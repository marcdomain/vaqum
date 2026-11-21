//! The `.vaqum` container format.
//!
//! Layout (all integers little-endian):
//! ```text
//! magic          4 bytes   b"VAQM"
//! version        1 byte    format version (currently 2)
//! entry_type     1 byte    0 = single file, 1 = directory archive (tar)
//! algorithm      1 byte    0 = zstd, 1 = xz/LZMA
//! flags          1 byte    bit 0 = dedup enabled, bit 1 = encrypted,
//!                          bit 2 = strong key derivation (see crypto.rs)
//! original_size  8 bytes   uncompressed size of the payload (file bytes, or
//!                          tar-stream bytes for a directory archive)
//! checksum       32 bytes  SHA-256 of the uncompressed payload
//! salt           16 bytes  Argon2id salt (zero if not encrypted)
//! nonce_prefix   7 bytes   ChaCha20-Poly1305 stream nonce prefix (zero if
//!                          not encrypted)
//! name_len       2 bytes   length in bytes of `name`
//! name           N bytes   UTF-8 original file/directory name
//! -- version 3 only (encrypted for recipients, `compress --to`):
//! recipients     1 byte    number of recipients (1-255)
//! stanzas        80 bytes  per recipient: ephemeral X25519 key + wrapped file key
//! mac            32 bytes  HMAC-SHA256 over every header byte before it
//! ---
//! payload, to end of file (compressed, then encrypted if flagged)
//! ```
//!
//! `compressed_size` isn't stored: it's `total file size - header.on_disk_len()`.
//! v1 files aren't readable by this build; `Header::read` bails with an
//! upgrade message rather than misreading them.

use std::io::{self, Read, Write};

use anyhow::{Context, Result, bail};

pub const MAGIC: &[u8; 4] = b"VAQM";
pub const VERSION: u8 = 2;
/// Archives encrypted for recipients (see `keys.rs`).
pub const VERSION_RECIPIENTS: u8 = 3;

pub const FLAG_DEDUP: u8 = 0b0000_0001;
pub const FLAG_ENCRYPTED: u8 = 0b0000_0010;
pub const FLAG_STRONG_KDF: u8 = 0b0000_0100;

const SALT_LEN: usize = crate::crypto::SALT_LEN;
const NONCE_PREFIX_LEN: usize = crate::crypto::NONCE_PREFIX_LEN;

/// Fixed-size portion of the header, before the variable-length name.
const FIXED_LEN: usize = 4 + 1 + 1 + 1 + 1 + 8 + 32 + SALT_LEN + NONCE_PREFIX_LEN + 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryType {
    File,
    Archive,
    /// Multiple top-level paths bundled together; unpacks directly into
    /// the output directory instead of nesting under `name`.
    Bundle,
}

impl EntryType {
    fn to_byte(self) -> u8 {
        match self {
            EntryType::File => 0,
            EntryType::Archive => 1,
            EntryType::Bundle => 2,
        }
    }

    fn from_byte(b: u8) -> Result<Self> {
        match b {
            0 => Ok(EntryType::File),
            1 => Ok(EntryType::Archive),
            2 => Ok(EntryType::Bundle),
            other => bail!("unsupported vaqum entry type byte: {other}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    Zstd,
    Xz,
}

impl Algorithm {
    fn to_byte(self) -> u8 {
        match self {
            Algorithm::Zstd => 0,
            Algorithm::Xz => 1,
        }
    }

    fn from_byte(b: u8) -> Result<Self> {
        match b {
            0 => Ok(Algorithm::Zstd),
            1 => Ok(Algorithm::Xz),
            other => bail!("unsupported vaqum algorithm byte: {other}"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Algorithm::Zstd => "zstd",
            Algorithm::Xz => "xz (LZMA)",
        }
    }
}

#[derive(Debug)]
pub struct Header {
    pub entry_type: EntryType,
    pub algorithm: Algorithm,
    pub dedup: bool,
    pub encrypted: bool,
    /// Key derived with the stronger Argon2id settings (set on every
    /// archive encrypted by this version; older archives use the defaults).
    pub strong_kdf: bool,
    pub original_size: u64,
    pub checksum: [u8; 32],
    pub salt: [u8; SALT_LEN],
    pub nonce_prefix: [u8; NONCE_PREFIX_LEN],
    pub name: String,
    /// Wrapped file keys, one per recipient (version 3 archives only).
    pub recipients: Vec<crate::keys::Stanza>,
    /// Header MAC (version 3 archives only).
    pub mac: [u8; crate::keys::MAC_LEN],
}

impl Header {
    pub fn write<W: Write>(&self, mut w: W) -> Result<()> {
        w.write_all(&self.bytes_without_mac()?)?;
        if !self.recipients.is_empty() {
            w.write_all(&self.mac)?;
        }
        Ok(())
    }

    /// Every header byte except the trailing MAC (what the MAC covers).
    pub fn bytes_without_mac(&self) -> Result<Vec<u8>> {
        let mut w =
            Vec::with_capacity(FIXED_LEN + self.name.len() + 1 + self.recipients.len() * 80);
        let version = if self.recipients.is_empty() {
            VERSION
        } else {
            VERSION_RECIPIENTS
        };
        w.write_all(MAGIC)?;
        w.write_all(&[version])?;
        w.write_all(&[self.entry_type.to_byte()])?;
        w.write_all(&[self.algorithm.to_byte()])?;
        let mut flags = if self.dedup { FLAG_DEDUP } else { 0 };
        if self.encrypted {
            flags |= FLAG_ENCRYPTED;
        }
        if self.strong_kdf {
            flags |= FLAG_STRONG_KDF;
        }
        w.write_all(&[flags])?;
        w.write_all(&self.original_size.to_le_bytes())?;
        w.write_all(&self.checksum)?;
        w.write_all(&self.salt)?;
        w.write_all(&self.nonce_prefix)?;
        let name_bytes = self.name.as_bytes();
        if name_bytes.len() > u16::MAX as usize {
            bail!("file name too long to store in vaqum header");
        }
        w.write_all(&(name_bytes.len() as u16).to_le_bytes())?;
        w.write_all(name_bytes)?;
        if !self.recipients.is_empty() {
            if self.recipients.len() > u8::MAX as usize {
                bail!("an archive can have at most 255 recipients");
            }
            w.write_all(&[self.recipients.len() as u8])?;
            for stanza in &self.recipients {
                w.write_all(stanza)?;
            }
        }
        Ok(w)
    }

    pub fn read<R: Read>(mut r: R) -> Result<Self> {
        let mut fixed = [0u8; FIXED_LEN];
        r.read_exact(&mut fixed)
            .context("failed to read vaqum header (file too short or not a vaqum file)")?;

        if &fixed[0..4] != MAGIC {
            bail!("not a vaqum file (bad magic bytes)");
        }
        let version = fixed[4];
        if version != VERSION && version != VERSION_RECIPIENTS {
            bail!(
                "unsupported vaqum format version {version} (this build supports version {VERSION}); re-compress the file with this version of vaqum"
            );
        }
        let entry_type = EntryType::from_byte(fixed[5])?;
        let algorithm = Algorithm::from_byte(fixed[6])?;
        let flags = fixed[7];
        let dedup = flags & FLAG_DEDUP != 0;
        let encrypted = flags & FLAG_ENCRYPTED != 0;
        let strong_kdf = flags & FLAG_STRONG_KDF != 0;

        let mut original_size_bytes = [0u8; 8];
        original_size_bytes.copy_from_slice(&fixed[8..16]);
        let original_size = u64::from_le_bytes(original_size_bytes);

        let mut checksum = [0u8; 32];
        checksum.copy_from_slice(&fixed[16..48]);

        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&fixed[48..48 + SALT_LEN]);
        let mut nonce_prefix = [0u8; NONCE_PREFIX_LEN];
        nonce_prefix.copy_from_slice(&fixed[48 + SALT_LEN..48 + SALT_LEN + NONCE_PREFIX_LEN]);

        let name_len_offset = 48 + SALT_LEN + NONCE_PREFIX_LEN;
        let mut name_len_bytes = [0u8; 2];
        name_len_bytes.copy_from_slice(&fixed[name_len_offset..name_len_offset + 2]);
        let name_len = u16::from_le_bytes(name_len_bytes) as usize;

        let mut name_buf = vec![0u8; name_len];
        r.read_exact(&mut name_buf)
            .context("failed to read vaqum header name field")?;
        let name = String::from_utf8(name_buf).context("vaqum header name is not valid UTF-8")?;
        if !is_plain_name(&name) {
            bail!(
                "refusing to open this archive: its stored name {name:?} isn't a plain file name \
                 (it could write outside the output directory)"
            );
        }

        let mut recipients = Vec::new();
        let mut mac = [0u8; crate::keys::MAC_LEN];
        if version == VERSION_RECIPIENTS {
            if !encrypted {
                bail!("corrupt vaqum header: recipient archive isn't marked encrypted");
            }
            let mut count = [0u8; 1];
            r.read_exact(&mut count)
                .context("failed to read the archive's recipient list")?;
            if count[0] == 0 {
                bail!("corrupt vaqum header: recipient archive has no recipients");
            }
            for _ in 0..count[0] {
                let mut stanza = [0u8; crate::keys::STANZA_LEN];
                r.read_exact(&mut stanza)
                    .context("failed to read the archive's recipient list")?;
                recipients.push(stanza);
            }
            r.read_exact(&mut mac)
                .context("failed to read the archive's header MAC")?;
        }

        Ok(Header {
            recipients,
            mac,
            entry_type,
            algorithm,
            dedup,
            encrypted,
            strong_kdf,
            original_size,
            checksum,
            salt,
            nonce_prefix,
            name,
        })
    }

    pub fn on_disk_len(&self) -> u64 {
        let recipients = if self.recipients.is_empty() {
            0
        } else {
            1 + self.recipients.len() as u64 * crate::keys::STANZA_LEN as u64
                + crate::keys::MAC_LEN as u64
        };
        FIXED_LEN as u64 + self.name.len() as u64 + recipients
    }
}

/// The stored name is joined onto the output directory on decompress, so
/// it must be a single, ordinary path component: no separators, no `..`.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['/', '\\', '\0'])
        && name != "."
        && name != ".."
        && std::path::Path::new(name).components().count() == 1
        && matches!(
            std::path::Path::new(name).components().next(),
            Some(std::path::Component::Normal(_))
        )
}

/// Errors if more bytes come out of a decoder than the archive header
/// promised. The header's size drives the decompression-bomb and
/// disk-space checks, so a lying header must not get past them.
pub struct SizeLimitReader<R: Read> {
    inner: R,
    remaining: u64,
}

impl<R: Read> SizeLimitReader<R> {
    pub fn new(inner: R, limit: u64) -> Self {
        Self {
            inner,
            remaining: limit,
        }
    }
}

impl<R: Read> Read for SizeLimitReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n as u64 > self.remaining {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "archive decompresses to more data than its header claims (possible decompression bomb); refusing to continue",
            ));
        }
        self.remaining -= n as u64;
        Ok(n)
    }
}

/// Peek at just the header of a `.vaqum` file, and return the total file
/// size alongside it, without touching the compressed payload.
pub fn read_header_and_total_size(path: &std::path::Path) -> Result<(Header, u64)> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let total_size = file.metadata()?.len();
    let header = Header::read(&mut file)?;
    Ok((header, total_size))
}

/// Detect a `.vaqum` file by its magic bytes, independent of extension.
pub fn is_vaqum_file(path: &std::path::Path) -> Result<bool> {
    let mut f =
        std::fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut magic = [0u8; 4];
    match f.read_exact(&mut magic) {
        Ok(()) => Ok(&magic == MAGIC),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// A `Write` wrapper that transparently hashes (SHA-256) and counts every
/// byte written through it, before forwarding to the inner writer.
pub struct HashingWriter<W: Write> {
    inner: W,
    hasher: sha2::Sha256,
    count: u64,
}

impl<W: Write> HashingWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: sha2::Digest::new(),
            count: 0,
        }
    }

    /// Consumes the wrapper, returning the inner writer plus the byte count
    /// and SHA-256 digest of everything written through it.
    pub fn into_inner_with_stats(self) -> (W, u64, [u8; 32]) {
        let digest = sha2::Digest::finalize(self.hasher);
        (self.inner, self.count, digest.into())
    }
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        sha2::Digest::update(&mut self.hasher, &buf[..n]);
        self.count += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// A `Read` wrapper that hashes (SHA-256) and counts every byte read
/// through it. Hashing can be switched off so callers that don't need a
/// checksum don't pay for one.
pub struct HashingReader<R: Read> {
    inner: R,
    hasher: Option<sha2::Sha256>,
    count: u64,
}

impl<R: Read> HashingReader<R> {
    pub fn new(inner: R, enabled: bool) -> Self {
        Self {
            inner,
            hasher: enabled.then(sha2::Digest::new),
            count: 0,
        }
    }

    /// Byte count and SHA-256 digest of everything read so far (the digest
    /// is all zeros if hashing was disabled).
    pub fn stats(self) -> (u64, [u8; 32]) {
        let digest = self
            .hasher
            .map(|h| sha2::Digest::finalize(h).into())
            .unwrap_or([0u8; 32]);
        (self.count, digest)
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if let Some(h) = &mut self.hasher {
            sha2::Digest::update(h, &buf[..n]);
        }
        self.count += n as u64;
        Ok(n)
    }
}

/// A `Write` wrapper that just counts bytes, used to measure compressed
/// output size during `--dry-run` without persisting anything.
pub struct CountingWriter<W: Write> {
    inner: W,
    count: u64,
}

impl<W: Write> CountingWriter<W> {
    pub fn new(inner: W) -> Self {
        Self { inner, count: 0 }
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.count += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
