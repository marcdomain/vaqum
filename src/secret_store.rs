//! Encrypted secrets files (`vaqum secrets`, `vaqum run`): an `.env` that is
//! safe to commit. Names and members stay readable, so diffs and reviews
//! still work; values are encrypted.
//!
//! ```text
//! # comments are free text (not authenticated)
//! VQM-SECRETS-1
//! member: vqm1…                  one per member (their public key)
//! key: <base64>                  the file key, wrapped for that member
//! NAME=vqm:<base64>              nonce (12 bytes) + ChaCha20-Poly1305 ciphertext
//! mac: <base64>                  HMAC-SHA256 over every line above
//! ```
//!
//! - A random 32-byte file key protects the file; it is wrapped for each
//!   member exactly like `compress --to` archives (X25519 + HKDF +
//!   ChaCha20-Poly1305), so only members can recover it.
//! - Each value is sealed with a key derived from the file key, and its
//!   name is bound in as associated data: a value can't be moved to
//!   another name without failing to decrypt.
//! - The MAC (keyed from the file key) covers the marker, members, wrapped
//!   keys and every entry in order, so nobody without the file key can add,
//!   drop, reorder or alter anything, or add themselves as a member.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use chacha20poly1305::aead::{Aead, KeyInit as _, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use ct_codecs::{Base64UrlSafeNoPadding, Decoder as _, Encoder as _};
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::keys::{self, FileKey, Identity, PublicKey, Stanza};

const MARKER: &str = "VQM-SECRETS-1";
const VALUE_INFO: &[u8] = b"vaqum/secrets/v1/value";
const MAC_INFO: &[u8] = b"vaqum/secrets/v1/mac";
const VALUE_AAD: &[u8] = b"vaqum-secret\0";
const PREFIX: &str = "vqm:";
/// Where `vaqum secrets` and `vaqum run` look by default.
pub const DEFAULT_FILE: &str = ".env.vqm";

fn b64(bytes: &[u8]) -> String {
    Base64UrlSafeNoPadding::encode_to_string(bytes).expect("encoding can't fail")
}

fn unb64(text: &str, what: &str) -> Result<Vec<u8>> {
    Base64UrlSafeNoPadding::decode_to_vec(text.trim(), None)
        .ok()
        .with_context(|| format!("secrets file has a damaged {what}"))
}

fn derive(file_key: &FileKey, info: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    Hkdf::<Sha256>::new(None, file_key)
        .expand(info, &mut out)
        .expect("32 bytes is a valid HKDF length");
    out
}

/// Environment-variable style names only: letters, digits, underscore.
pub fn check_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name.len() <= 256
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        bail!(
            "'{name}' isn't a valid secret name (use letters, digits and _, not starting with a digit)"
        );
    }
    Ok(())
}

pub struct Member {
    pub public: String,
    stanza: Stanza,
}

/// A secrets file as stored: readable structure, encrypted values.
pub struct SecretsFile {
    pub members: Vec<Member>,
    /// (name, nonce + ciphertext), in file order.
    entries: Vec<(String, Vec<u8>)>,
    mac: [u8; 32],
}

/// A secrets file unlocked with a member's key.
pub struct Unlocked {
    pub file: SecretsFile,
    file_key: FileKey,
}

impl SecretsFile {
    /// Parses and structurally validates a secrets file (no key needed).
    pub fn parse(text: &str) -> Result<Self> {
        let mut lines = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'));
        if lines.next() != Some(MARKER) {
            bail!("not a vaqum secrets file (create one with `vaqum secrets init`)");
        }
        let (mut members, mut entries, mut mac) = (Vec::new(), Vec::new(), None);
        let mut pending_member: Option<String> = None;
        for line in lines {
            if mac.is_some() {
                bail!("secrets file has content after its MAC line");
            }
            if let Some(public) = line.strip_prefix("member:") {
                if pending_member.is_some() {
                    bail!("secrets file has a member without a key");
                }
                PublicKey::parse(public.trim())?;
                pending_member = Some(public.trim().to_string());
            } else if let Some(key) = line.strip_prefix("key:") {
                let public = pending_member
                    .take()
                    .context("secrets file has a key without a member")?;
                let stanza: Stanza = unb64(key, "wrapped key")?
                    .try_into()
                    .ok()
                    .context("secrets file has a wrapped key of the wrong length")?;
                members.push(Member { public, stanza });
            } else if let Some(tag) = line.strip_prefix("mac:") {
                mac = Some(
                    <[u8; 32]>::try_from(unb64(tag, "MAC")?)
                        .ok()
                        .context("secrets file MAC has the wrong length")?,
                );
            } else if let Some((name, value)) = line.split_once('=') {
                check_name(name)?;
                if entries.iter().any(|(n, _)| n == name) {
                    bail!("secrets file lists {name} twice");
                }
                let sealed = value
                    .strip_prefix(PREFIX)
                    .with_context(|| format!("{name} isn't encrypted (expected {PREFIX}…)"))?;
                let sealed = unb64(sealed, "value")?;
                if sealed.len() < 12 + 16 {
                    bail!("secrets file has a truncated value for {name}");
                }
                entries.push((name.to_string(), sealed));
            } else {
                bail!("secrets file has an unrecognized line: {line:.40}");
            }
        }
        if pending_member.is_some() {
            bail!("secrets file has a member without a key");
        }
        if members.is_empty() {
            bail!("secrets file has no members");
        }
        let mac = mac.context("secrets file is missing its MAC (it may be truncated)")?;
        Ok(Self {
            members,
            entries,
            mac,
        })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).with_context(|| {
            format!(
                "failed to read {} (create it with `vaqum secrets init`)",
                path.display()
            )
        })?;
        Self::parse(&text).with_context(|| format!("in {}", path.display()))
    }

    /// Secret names, in file order (readable without a key).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(n, _)| n.as_str())
    }

    /// The exact bytes the MAC covers.
    fn authenticated_text(&self) -> String {
        let mut text = format!("{MARKER}\n");
        for m in &self.members {
            text.push_str(&format!("member: {}\nkey: {}\n", m.public, b64(&m.stanza)));
        }
        for (name, sealed) in &self.entries {
            text.push_str(&format!("{name}={PREFIX}{}\n", b64(sealed)));
        }
        text
    }

    /// Recovers the file key with `identity` and checks the MAC.
    pub fn unlock(self, identity: &Identity) -> Result<Unlocked> {
        let stanzas: Vec<Stanza> = self.members.iter().map(|m| m.stanza).collect();
        let file_key = keys::unwrap(&stanzas, identity).map_err(|_| {
            anyhow::anyhow!("you're not a member of this secrets file; ask a member to run `vaqum secrets add-member <your public key>`")
        })?;
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&derive(&file_key, MAC_INFO))
            .expect("HMAC accepts any key");
        mac.update(self.authenticated_text().as_bytes());
        mac.verify_slice(&self.mac).map_err(|_| {
            anyhow::anyhow!("the secrets file has been tampered with (its MAC doesn't match)")
        })?;
        Ok(Unlocked {
            file: self,
            file_key,
        })
    }

    /// A new, empty secrets file for these members.
    pub fn create(members: &[PublicKey]) -> Result<Unlocked> {
        let file_key = keys::new_file_key();
        let file = SecretsFile {
            members: wrap_for(&file_key, members)?,
            entries: Vec::new(),
            mac: [0; 32],
        };
        Ok(Unlocked { file, file_key })
    }
}

fn wrap_for(file_key: &FileKey, members: &[PublicKey]) -> Result<Vec<Member>> {
    let stanzas = keys::wrap(file_key, members)?;
    Ok(members
        .iter()
        .zip(stanzas)
        .map(|(pk, stanza)| Member {
            public: pk.encode(),
            stanza,
        })
        .collect())
}

impl Unlocked {
    fn cipher(&self) -> ChaCha20Poly1305 {
        ChaCha20Poly1305::new(&derive(&self.file_key, VALUE_INFO).into())
    }

    fn aad(name: &str) -> Vec<u8> {
        [VALUE_AAD, name.as_bytes()].concat()
    }

    pub fn get(&self, name: &str) -> Result<Option<String>> {
        let Some((_, sealed)) = self.file.entries.iter().find(|(n, _)| n == name) else {
            return Ok(None);
        };
        let (nonce, ciphertext) = sealed.split_at(12);
        let plain = self
            .cipher()
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: &Self::aad(name),
                },
            )
            .map_err(|_| anyhow::anyhow!("the value of {name} is damaged or was tampered with"))?;
        Ok(Some(
            String::from_utf8(plain).context("secret value isn't valid text")?,
        ))
    }

    /// Every secret, decrypted, in file order.
    pub fn all(&self) -> Result<Vec<(String, String)>> {
        self.file
            .entries
            .iter()
            .map(|(n, _)| Ok((n.clone(), self.get(n)?.expect("listed names exist"))))
            .collect()
    }

    /// Sets (adds or replaces) a secret.
    pub fn set(&mut self, name: &str, value: &str) -> Result<()> {
        check_name(name)?;
        if value.contains('\0') {
            bail!("secret values can't contain NUL bytes");
        }
        let mut nonce = [0u8; 12];
        rand::Rng::fill_bytes(&mut rand::rng(), &mut nonce);
        let ciphertext = self
            .cipher()
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: value.as_bytes(),
                    aad: &Self::aad(name),
                },
            )
            .map_err(|_| anyhow::anyhow!("failed to encrypt {name}"))?;
        let sealed = [nonce.as_slice(), &ciphertext].concat();
        match self.file.entries.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 = sealed,
            None => self.file.entries.push((name.to_string(), sealed)),
        }
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.file.entries.len();
        self.file.entries.retain(|(n, _)| n != name);
        self.file.entries.len() != before
    }

    pub fn add_member(&mut self, member: &PublicKey) -> Result<bool> {
        let public = member.encode();
        if self.file.members.iter().any(|m| m.public == public) {
            return Ok(false);
        }
        self.file
            .members
            .extend(wrap_for(&self.file_key, std::slice::from_ref(member))?);
        Ok(true)
    }

    /// Removes a member and re-encrypts everything under a new file key, so
    /// the removed member can't read future versions of the file.
    pub fn remove_member(&mut self, member: &PublicKey) -> Result<bool> {
        let public = member.encode();
        if !self.file.members.iter().any(|m| m.public == public) {
            return Ok(false);
        }
        if self.file.members.len() == 1 {
            bail!("can't remove the last member: nobody could open the file");
        }
        let secrets = self.all()?;
        let remaining: Vec<PublicKey> = self
            .file
            .members
            .iter()
            .filter(|m| m.public != public)
            .map(|m| PublicKey::parse(&m.public))
            .collect::<Result<_>>()?;
        self.file_key = keys::new_file_key();
        self.file.members = wrap_for(&self.file_key, &remaining)?;
        self.file.entries.clear();
        for (name, value) in secrets {
            self.set(&name, &value)?;
        }
        Ok(true)
    }

    /// The full file text, with a fresh MAC.
    pub fn render(&mut self) -> String {
        let body = self.file.authenticated_text();
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&derive(&self.file_key, MAC_INFO))
            .expect("HMAC accepts any key");
        mac.update(body.as_bytes());
        self.file.mac = mac.finalize().into_bytes().into();
        format!(
            "# vaqum secrets: names and members are readable, values are encrypted.\n\
             # Change with `vaqum secrets set|rm|import`; run apps with `vaqum run -- <command>`.\n\
             {body}mac: {}\n",
            b64(&self.file.mac)
        )
    }

    /// Writes the file atomically.
    pub fn save(&mut self, path: &Path) -> Result<()> {
        let text = self.render();
        let mut out = crate::util::AtomicFile::create(path)?;
        std::io::Write::write_all(out.file(), text.as_bytes())?;
        out.commit()
    }
}

/// Parses `.env` text into (name, value) pairs: `NAME=value`, optional
/// `export`, single or double quotes, `#` comments and blank lines.
pub fn parse_dotenv(text: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (name, value) = line
            .split_once('=')
            .with_context(|| format!("line {}: expected NAME=value", i + 1))?;
        let name = name.trim();
        check_name(name).with_context(|| format!("line {}", i + 1))?;
        let value = value.trim();
        let value = if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
            let inner = &value[1..value.len() - 1];
            if value.starts_with('"') {
                inner
                    .replace("\\n", "\n")
                    .replace("\\\"", "\"")
                    .replace("\\\\", "\\")
            } else {
                inner.to_string()
            }
        } else {
            value
                .split(" #")
                .next()
                .unwrap_or("")
                .trim_end()
                .to_string()
        };
        out.push((name.to_string(), value));
    }
    Ok(out)
}

/// Formats a value for a `.env` line, quoting when needed.
pub fn quote_dotenv(value: &str) -> String {
    let plain = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:@%+,=".contains(c));
    if plain {
        value.to_string()
    } else {
        format!(
            "\"{}\"",
            value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
        )
    }
}
