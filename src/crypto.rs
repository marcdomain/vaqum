//! Compress-then-encrypt: ChaCha20-Poly1305 (AEAD) with an Argon2id-derived
//! key. The key itself is never stored — only the salt needed to re-derive
//! it from a password/keyfile. Streams in fixed-size chunks so large
//! archives never need to be fully buffered in memory.

use std::io::{self, Read, Write};

use anyhow::{Context, Result, bail};
use argon2::Argon2;
use chacha20poly1305::ChaCha20Poly1305;
use chacha20poly1305::aead::stream::{DecryptorBE32, EncryptorBE32};

pub const SALT_LEN: usize = 16;
pub const NONCE_PREFIX_LEN: usize = 7;
const CHUNK_SIZE: usize = 64 * 1024;
const TAG_LEN: usize = 16;

pub type Salt = [u8; SALT_LEN];
pub type NoncePrefix = [u8; NONCE_PREFIX_LEN];
pub type Key = chacha20poly1305::Key;

pub fn derive_key(password: &[u8], salt: &Salt) -> Result<Key> {
    let mut bytes = [0u8; 32];
    Argon2::default()
        .hash_password_into(password, salt, &mut bytes)
        .map_err(|e| anyhow::anyhow!("key derivation failed: {e}"))?;
    Ok(Key::from(bytes))
}

pub fn random_salt() -> Salt {
    let mut salt = [0u8; SALT_LEN];
    getrandom(&mut salt);
    salt
}

pub fn random_nonce_prefix() -> NoncePrefix {
    let mut nonce = [0u8; NONCE_PREFIX_LEN];
    getrandom(&mut nonce);
    nonce
}

fn getrandom(buf: &mut [u8]) {
    use rand::RngExt;
    rand::rng().fill(buf);
}

/// Call [`finish`](Self::finish) when done — it encrypts whatever remains
/// (even if empty) as the final, authenticated chunk.
pub struct Encryptor<W: Write> {
    inner: W,
    stream: EncryptorBE32<ChaCha20Poly1305>,
    buf: Vec<u8>,
}

impl<W: Write> Encryptor<W> {
    pub fn new(inner: W, key: &Key, nonce_prefix: &NoncePrefix) -> Self {
        let stream = EncryptorBE32::new(key, nonce_prefix.into());
        Self {
            inner,
            stream,
            buf: Vec::with_capacity(CHUNK_SIZE),
        }
    }

    pub fn finish(mut self) -> Result<W> {
        let ciphertext = self
            .stream
            .encrypt_last(self.buf.as_slice())
            .map_err(|_| anyhow::anyhow!("encryption failed"))?;
        self.inner
            .write_all(&ciphertext)
            .context("failed to write final encrypted chunk")?;
        Ok(self.inner)
    }
}

impl<W: Write> Write for Encryptor<W> {
    fn write(&mut self, mut data: &[u8]) -> io::Result<usize> {
        let total = data.len();
        while !data.is_empty() {
            let space = CHUNK_SIZE - self.buf.len();
            let take = space.min(data.len());
            self.buf.extend_from_slice(&data[..take]);
            data = &data[take..];

            if self.buf.len() == CHUNK_SIZE {
                let ciphertext = self
                    .stream
                    .encrypt_next(self.buf.as_slice())
                    .map_err(|_| io::Error::other("encryption failed"))?;
                self.inner.write_all(&ciphertext)?;
                self.buf.clear();
            }
        }
        Ok(total)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Reads one chunk ahead so it knows whether the chunk it's about to
/// decrypt is the last one — the AEAD stream needs that to detect
/// truncation.
pub struct Decryptor<R: Read> {
    inner: R,
    stream: Option<DecryptorBE32<ChaCha20Poly1305>>,
    pending: Option<Vec<u8>>,
    plaintext: Vec<u8>,
    plaintext_pos: usize,
}

impl<R: Read> Decryptor<R> {
    pub fn new(inner: R, key: &Key, nonce_prefix: &NoncePrefix) -> Self {
        let stream = DecryptorBE32::new(key, nonce_prefix.into());
        Self {
            inner,
            stream: Some(stream),
            pending: None,
            plaintext: Vec::new(),
            plaintext_pos: 0,
        }
    }

    fn read_ciphertext_chunk(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut buf = vec![0u8; CHUNK_SIZE + TAG_LEN];
        let mut filled = 0;
        while filled < buf.len() {
            let n = self.inner.read(&mut buf[filled..])?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled == 0 {
            return Ok(None);
        }
        buf.truncate(filled);
        Ok(Some(buf))
    }

    fn fill_plaintext(&mut self) -> io::Result<()> {
        if self.plaintext_pos < self.plaintext.len() {
            return Ok(());
        }
        let Some(mut stream) = self.stream.take() else {
            return Ok(());
        };

        let current = match self.pending.take() {
            Some(c) => c,
            None => match self.read_ciphertext_chunk()? {
                Some(c) => c,
                None => return Ok(()), // no ciphertext at all
            },
        };
        let next = self.read_ciphertext_chunk()?;
        let wrong_key_err = || {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "decryption failed: wrong password/keyfile, or the archive is corrupted or truncated",
            )
        };

        let plaintext = if let Some(next_chunk) = next {
            let pt = stream
                .decrypt_next(current.as_slice())
                .map_err(|_| wrong_key_err())?;
            self.stream = Some(stream);
            self.pending = Some(next_chunk);
            pt
        } else {
            stream
                .decrypt_last(current.as_slice())
                .map_err(|_| wrong_key_err())?
        };

        self.plaintext = plaintext;
        self.plaintext_pos = 0;
        Ok(())
    }
}

impl<R: Read> Read for Decryptor<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.fill_plaintext()?;
        let available = &self.plaintext[self.plaintext_pos..];
        if available.is_empty() {
            return Ok(0);
        }
        let n = available.len().min(out.len());
        out[..n].copy_from_slice(&available[..n]);
        self.plaintext_pos += n;
        Ok(n)
    }
}

/// Wraps a writer with encryption only when key material is present,
/// so callers can build one pipeline regardless of whether `-e`/`--key-file`
/// was passed.
pub enum EncryptWriter<W: Write> {
    Plain(W),
    Encrypted(Encryptor<W>),
}

impl<W: Write> EncryptWriter<W> {
    pub fn new(inner: W, key: Option<(&Key, &NoncePrefix)>) -> Self {
        match key {
            Some((key, nonce_prefix)) => Self::Encrypted(Encryptor::new(inner, key, nonce_prefix)),
            None => Self::Plain(inner),
        }
    }

    pub fn finish(self) -> Result<W> {
        match self {
            Self::Plain(w) => Ok(w),
            Self::Encrypted(e) => e.finish(),
        }
    }
}

impl<W: Write> Write for EncryptWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(w) => w.write(buf),
            Self::Encrypted(e) => e.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(w) => w.flush(),
            Self::Encrypted(e) => e.flush(),
        }
    }
}

/// The read-side counterpart of [`EncryptWriter`].
pub enum DecryptReader<R: Read> {
    Plain(R),
    Encrypted(Decryptor<R>),
}

impl<R: Read> DecryptReader<R> {
    pub fn new(inner: R, key: Option<(&Key, &NoncePrefix)>) -> Self {
        match key {
            Some((key, nonce_prefix)) => Self::Encrypted(Decryptor::new(inner, key, nonce_prefix)),
            None => Self::Plain(inner),
        }
    }
}

impl<R: Read> Read for DecryptReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(r) => r.read(buf),
            Self::Encrypted(d) => d.read(buf),
        }
    }
}

pub fn prompt_new_password() -> Result<String> {
    let password =
        rpassword::prompt_password("Enter password: ").context("failed to read password")?;
    let confirm =
        rpassword::prompt_password("Confirm password: ").context("failed to read password")?;
    if password != confirm {
        bail!("passwords did not match");
    }
    warn_if_weak(&password);
    Ok(password)
}

pub fn prompt_existing_password() -> Result<String> {
    rpassword::prompt_password("Enter password: ").context("failed to read password")
}

fn warn_if_weak(password: &str) {
    let estimate = zxcvbn::zxcvbn(password, &[]);
    if estimate.score() < zxcvbn::Score::Three {
        eprintln!(
            "⚠  This password looks weak (crack time estimate: {}). Consider a longer or more random one.",
            estimate.crack_times().offline_slow_hashing_1e4_per_second()
        );
    }
}
