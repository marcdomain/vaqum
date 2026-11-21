//! Personal keys: encrypt an archive *for* someone (no shared password)
//! and sign archives so recipients can check who made them.
//!
//! - **Identity** (secret, `vaqum keygen`): an X25519 key for decryption and
//!   an Ed25519 (minisign) key for signing, in one file.
//! - **Public key** (shareable): one `vqm1…` string holding both public
//!   halves plus a checksum, so it can be pasted into a chat or a README.
//!
//! Encrypting for recipients follows age's X25519 recipient design
//! (https://age-encryption.org/v1): a random 32-byte file key encrypts the
//! payload; for each recipient it is wrapped with ChaCha20-Poly1305 under a
//! key derived (HKDF-SHA256) from an ephemeral X25519 exchange. A MAC keyed
//! from the file key covers the whole header, so a recipient list, name or
//! size can't be altered without detection.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chacha20poly1305::aead::{Aead, KeyInit as _};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use ct_codecs::{Base64UrlSafeNoPadding, Decoder as _, Encoder as _};
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey as XPublic, StaticSecret};

use crate::crypto::Key;

const PUBLIC_PREFIX: &str = "vqm1";
const IDENTITY_MARKER: &str = "VAQUM-IDENTITY-1";
const MINISIGN_PK_LEN: usize = 42;
const CHECKSUM_LEN: usize = 4;

/// Bytes per recipient in an archive header: ephemeral public key plus
/// the wrapped file key (32 bytes + 16-byte tag).
pub const STANZA_LEN: usize = 32 + 48;
pub const MAC_LEN: usize = 32;

const WRAP_INFO: &[u8] = b"vaqum/v3/x25519";
const PAYLOAD_INFO: &[u8] = b"vaqum/v3/payload";
const MAC_INFO: &[u8] = b"vaqum/v3/header-mac";

pub type Stanza = [u8; STANZA_LEN];
pub type FileKey = [u8; 32];

fn random32() -> [u8; 32] {
    let mut b = [0u8; 32];
    rand::Rng::fill_bytes(&mut rand::rng(), &mut b);
    b
}

fn hkdf(ikm: &[u8], salt: &[u8], info: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    Hkdf::<Sha256>::new(Some(salt), ikm)
        .expand(info, &mut out)
        .expect("32 bytes is a valid HKDF output length");
    out
}

// ------------------------------------------------------------ public keys

/// Someone's shareable public key.
#[derive(Clone)]
pub struct PublicKey {
    x25519: [u8; 32],
    signing: minisign::PublicKey,
}

impl PublicKey {
    /// The `vqm1…` text form.
    pub fn encode(&self) -> String {
        let mut raw = Vec::with_capacity(32 + MINISIGN_PK_LEN + CHECKSUM_LEN);
        raw.extend_from_slice(&self.x25519);
        raw.extend_from_slice(&self.signing.to_bytes());
        let check = Sha256::digest(&raw);
        raw.extend_from_slice(&check[..CHECKSUM_LEN]);
        let b64 = Base64UrlSafeNoPadding::encode_to_string(&raw).expect("encoding can't fail");
        format!("{PUBLIC_PREFIX}{b64}")
    }

    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        let body = s.strip_prefix(PUBLIC_PREFIX).with_context(|| {
            format!("not a vaqum public key (should start with {PUBLIC_PREFIX})")
        })?;
        let raw = Base64UrlSafeNoPadding::decode_to_vec(body, None)
            .ok()
            .filter(|r| r.len() == 32 + MINISIGN_PK_LEN + CHECKSUM_LEN)
            .context("this vaqum public key is damaged or incomplete")?;
        let (key, check) = raw.split_at(32 + MINISIGN_PK_LEN);
        if Sha256::digest(key)[..CHECKSUM_LEN] != *check {
            bail!("this vaqum public key has a typo (its checksum doesn't match)");
        }
        let mut x25519 = [0u8; 32];
        x25519.copy_from_slice(&key[..32]);
        let signing = minisign::PublicKey::from_bytes(&key[32..])
            .map_err(|e| anyhow::anyhow!("invalid signing key in public key: {e}"))?;
        Ok(Self { x25519, signing })
    }

    /// Accepts a `vqm1…` string, or a path to a file containing one.
    pub fn load(arg: &str) -> Result<Self> {
        if arg.trim_start().starts_with(PUBLIC_PREFIX) {
            return Self::parse(arg);
        }
        let text = fs::read_to_string(arg).with_context(|| {
            format!("'{arg}' is neither a vqm1… public key nor a readable file")
        })?;
        let line = text
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with(PUBLIC_PREFIX))
            .with_context(|| format!("no vqm1… public key found in {arg}"))?;
        Self::parse(line)
    }

    pub fn signing_key(&self) -> &minisign::PublicKey {
        &self.signing
    }
}

// --------------------------------------------------------------- identity

/// Your secret keys.
pub struct Identity {
    x25519: StaticSecret,
    signing: minisign::SecretKey,
    public: PublicKey,
}

impl Identity {
    pub fn generate() -> Result<Self> {
        let x25519 = StaticSecret::from(random32());
        let pair = minisign::KeyPair::generate_unencrypted_keypair()
            .map_err(|e| anyhow::anyhow!("failed to generate a signing key: {e}"))?;
        let public = PublicKey {
            x25519: XPublic::from(&x25519).to_bytes(),
            signing: pair.pk,
        };
        Ok(Self {
            x25519,
            signing: pair.sk,
            public,
        })
    }

    pub fn public(&self) -> &PublicKey {
        &self.public
    }

    pub fn signing_key(&self) -> &minisign::SecretKey {
        &self.signing
    }

    fn to_text(&self) -> String {
        let enc = Base64UrlSafeNoPadding::encode_to_string(self.x25519.to_bytes())
            .expect("encoding can't fail");
        let sign = Base64UrlSafeNoPadding::encode_to_string(self.signing.to_bytes())
            .expect("encoding can't fail");
        format!(
            "# vaqum identity: KEEP THIS FILE SECRET.\n\
             # Anyone with it can open archives sent to you and sign archives as you.\n\
             # Your public key (safe to share): {}\n\
             {IDENTITY_MARKER}\n\
             encryption: {enc}\n\
             signing: {sign}\n",
            self.public.encode()
        )
    }

    fn from_text(text: &str) -> Result<Self> {
        if !text.lines().any(|l| l.trim() == IDENTITY_MARKER) {
            bail!("not a vaqum identity file (create one with `vaqum keygen`)");
        }
        let field = |name: &str| -> Result<Vec<u8>> {
            let value = text
                .lines()
                .find_map(|l| l.trim().strip_prefix(&format!("{name}:")))
                .with_context(|| format!("identity file is missing its {name} key"))?;
            Base64UrlSafeNoPadding::decode_to_vec(value.trim(), None)
                .ok()
                .with_context(|| format!("identity file's {name} key is damaged"))
        };
        let enc: [u8; 32] = field("encryption")?
            .try_into()
            .ok()
            .context("identity file's encryption key has the wrong length")?;
        let x25519 = StaticSecret::from(enc);
        let signing = minisign::SecretKey::from_bytes(&field("signing")?)
            .map_err(|e| anyhow::anyhow!("identity file's signing key is invalid: {e}"))?;
        let signing_pk = minisign::PublicKey::from_secret_key(&signing)
            .map_err(|e| anyhow::anyhow!("identity file's signing key is invalid: {e}"))?;
        let public = PublicKey {
            x25519: XPublic::from(&x25519).to_bytes(),
            signing: signing_pk,
        };
        Ok(Self {
            x25519,
            signing,
            public,
        })
    }

    /// Writes the identity, readable only by you, refusing to overwrite.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(path).with_context(|| {
            if path.exists() {
                format!(
                    "{} already exists; refusing to overwrite a key (pass a different -o path)",
                    path.display()
                )
            } else {
                format!("failed to create {}", path.display())
            }
        })?;
        std::io::Write::write_all(&mut file, self.to_text().as_bytes())?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).with_context(|| {
            format!(
                "failed to read identity {} (create one with `vaqum keygen`)",
                path.display()
            )
        })?;
        Self::from_text(&text).with_context(|| format!("in {}", path.display()))
    }

    /// `path`, or the default identity location.
    pub fn load_or_default(path: Option<&Path>) -> Result<Self> {
        match path {
            Some(p) => Self::load(p),
            None => Self::load(&default_identity_path()?),
        }
    }
}

/// Where `vaqum keygen` puts your identity by default
/// (`~/.config/vaqum/identity`, `%APPDATA%\vaqum\identity` on Windows).
pub fn default_identity_path() -> Result<PathBuf> {
    Ok(crate::config::config_dir()?.join("identity"))
}

// ------------------------------------------------- recipient encryption

/// A fresh random key for one archive.
pub fn new_file_key() -> FileKey {
    random32()
}

/// The payload encryption key derived from the file key.
pub fn payload_key(file_key: &FileKey) -> Key {
    Key::from(hkdf(file_key, b"", PAYLOAD_INFO))
}

/// Wraps the file key for each recipient.
pub fn wrap(file_key: &FileKey, recipients: &[PublicKey]) -> Result<Vec<Stanza>> {
    recipients
        .iter()
        .map(|r| {
            let recipient = XPublic::from(r.x25519);
            let ephemeral = StaticSecret::from(random32());
            let ephemeral_pub = XPublic::from(&ephemeral).to_bytes();
            let shared = ephemeral.diffie_hellman(&recipient);
            if !shared.was_contributory() {
                bail!("refusing to encrypt for an invalid public key");
            }
            let wrapped = seal_file_key(shared.as_bytes(), &ephemeral_pub, &r.x25519, file_key);
            let mut stanza = [0u8; STANZA_LEN];
            stanza[..32].copy_from_slice(&ephemeral_pub);
            stanza[32..].copy_from_slice(&wrapped);
            Ok(stanza)
        })
        .collect()
}

/// Recovers the file key with `identity`, if the archive was encrypted for it.
pub fn unwrap(stanzas: &[Stanza], identity: &Identity) -> Result<FileKey> {
    let our_pub = identity.public.x25519;
    for stanza in stanzas {
        let mut ephemeral_pub = [0u8; 32];
        ephemeral_pub.copy_from_slice(&stanza[..32]);
        let shared = identity
            .x25519
            .diffie_hellman(&XPublic::from(ephemeral_pub));
        if !shared.was_contributory() {
            continue;
        }
        let wrap_key = hkdf(
            shared.as_bytes(),
            &[ephemeral_pub, our_pub].concat(),
            WRAP_INFO,
        );
        let cipher = ChaCha20Poly1305::new(&wrap_key.into());
        if let Ok(key) = cipher.decrypt(&Nonce::default(), &stanza[32..]) {
            return key
                .try_into()
                .ok()
                .context("archive holds a malformed file key");
        }
    }
    bail!(
        "this archive wasn't encrypted for your key ({} recipient{}); ask the sender to add your public key with --to",
        stanzas.len(),
        if stanzas.len() == 1 { "" } else { "s" }
    )
}

fn seal_file_key(
    shared: &[u8; 32],
    ephemeral_pub: &[u8; 32],
    recipient: &[u8; 32],
    file_key: &FileKey,
) -> Vec<u8> {
    let wrap_key = hkdf(shared, &[*ephemeral_pub, *recipient].concat(), WRAP_INFO);
    // The wrap key is unique to this exchange, so a zero nonce is safe.
    ChaCha20Poly1305::new(&wrap_key.into())
        .encrypt(&Nonce::default(), file_key.as_slice())
        .expect("encrypting 32 bytes can't fail")
}

/// MAC over the header bytes, keyed from the file key.
pub fn header_mac(file_key: &FileKey, header_bytes: &[u8]) -> [u8; MAC_LEN] {
    let key = hkdf(file_key, b"", MAC_INFO);
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&key).expect("HMAC accepts any key");
    mac.update(header_bytes);
    mac.finalize().into_bytes().into()
}

/// Constant-time check of a header MAC.
pub fn verify_header_mac(
    file_key: &FileKey,
    header_bytes: &[u8],
    expected: &[u8; MAC_LEN],
) -> Result<()> {
    let key = hkdf(file_key, b"", MAC_INFO);
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&key).expect("HMAC accepts any key");
    mac.update(header_bytes);
    mac.verify_slice(expected).map_err(|_| {
        anyhow::anyhow!("archive header has been tampered with (its MAC doesn't match)")
    })
}
