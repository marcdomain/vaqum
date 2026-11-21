//! Detached archive signatures (minisign format): `archive.vqm.minisig`
//! next to the archive. Anyone with the signer's `vqm1…` public key can
//! check who made the archive and that it hasn't been changed since.

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::keys::{Identity, PublicKey};
use crate::util::AtomicFile;

/// Where an archive's signature lives.
pub fn signature_path(archive: &Path) -> PathBuf {
    let mut s = archive.as_os_str().to_os_string();
    s.push(".minisig");
    PathBuf::from(s)
}

/// Signs `archive`, writing `<archive>.minisig`. Returns the signature path.
pub fn sign_file(archive: &Path, identity: &Identity) -> Result<PathBuf> {
    let file =
        File::open(archive).with_context(|| format!("failed to open {}", archive.display()))?;
    let bar = crate::progress::bar(
        file.metadata().map(|m| m.len()).unwrap_or(0),
        false,
        "Signing",
    );
    let file = crate::progress::ProgressReader::new(file, bar.clone());
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let trusted = format!(
        "timestamp:{timestamp}\tfile:{name}\tsigner:{}",
        identity.public().encode()
    );
    let signature = minisign::sign(
        Some(identity.public().signing_key()),
        identity.signing_key(),
        file,
        Some(&trusted),
        Some("signed with vaqum (minisign format)"),
    )
    .map_err(|e| anyhow::anyhow!("signing failed: {e}"))?;
    bar.finish_and_clear();

    let path = signature_path(archive);
    let mut out = AtomicFile::create(&path)?;
    std::io::Write::write_all(out.file(), signature.to_string().as_bytes())?;
    out.commit()?;
    Ok(path)
}

/// Checks `archive` against its signature and the expected signer.
/// Returns the signature's trusted comment (timestamp, file name).
pub fn verify_file(archive: &Path, signature: Option<&Path>, signer: &PublicKey) -> Result<String> {
    let sig_path = signature
        .map(Path::to_path_buf)
        .unwrap_or_else(|| signature_path(archive));
    let signature = minisign::SignatureBox::from_file(&sig_path).map_err(|e| {
        anyhow::anyhow!(
            "no readable signature at {} ({e}); the sender signs with `vaqum sign` or `compress --sign`",
            sig_path.display()
        )
    })?;
    let file =
        File::open(archive).with_context(|| format!("failed to open {}", archive.display()))?;
    let bar = crate::progress::bar(
        file.metadata().map(|m| m.len()).unwrap_or(0),
        false,
        "Checking signature",
    );
    let file = crate::progress::ProgressReader::new(file, bar.clone());
    let checked = minisign::verify(signer.signing_key(), &signature, file, true, false, false);
    bar.finish_and_clear();
    checked.map_err(|_| {
        anyhow::anyhow!(
            "signature check FAILED for {}: it wasn't signed by this key, or it was changed after signing",
            archive.display()
        )
    })?;
    signature
        .trusted_comment()
        .map_err(|e| anyhow::anyhow!("signature has an unreadable comment: {e}"))
}
