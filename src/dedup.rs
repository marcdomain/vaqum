//! Content-addressed deduplication for `vaqum compress -r --dedup`.
//!
//! Rather than mixing real file paths and dedup bookkeeping in the same
//! tar namespace (which risks collisions with a user's own files), a
//! dedup archive stores *only* two things inside the tar stream:
//!
//! - `.vaqum-objects/<sha256>` — one copy of each distinct file's bytes
//! - `.vaqum-manifest.json`    — every original path, directory, and which
//!   object hash it maps to
//!
//! Real paths are never written into the tar directly, so there is no way
//! for a source file to collide with vaqum's own bookkeeping names.

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::exclude::ExcludeSet;
use crate::util::{hash_tree, hex_encode, set_unix_mode};

pub const MANIFEST_ENTRY: &str = ".vaqum-manifest.json";
pub const OBJECTS_DIR: &str = ".vaqum-objects";

#[derive(Serialize, Deserialize)]
pub struct Manifest {
    /// Relative directory paths to (re)create, including empty ones.
    pub dirs: Vec<String>,
    pub files: Vec<FileEntry>,
}

#[derive(Serialize, Deserialize)]
pub struct FileEntry {
    /// Relative path, posix separators.
    pub path: String,
    /// Hex-encoded SHA-256 of the file's content; also its object key.
    pub hash: String,
    /// Unix permission bits, when available.
    pub mode: Option<u32>,
}

/// Build the dedup archive body (manifest + deduplicated objects) into
/// `builder`. Files with identical content are stored exactly once.
///
/// `on_file` is invoked once per source file as `(relative_path, size,
/// was_duplicate)`, useful for `-v/--verbose` progress output.
pub fn write_dedup_tar<W: Write>(
    root: &Path,
    threads: usize,
    exclude: Option<&ExcludeSet>,
    builder: &mut tar::Builder<W>,
    mut on_file: impl FnMut(&str, u64, bool),
) -> Result<()> {
    let (dirs, files) = hash_tree(root, threads, exclude)?;

    let mut manifest = Manifest {
        dirs,
        files: Vec::with_capacity(files.len()),
    };
    let mut seen_hashes: HashSet<String> = HashSet::new();

    for file in files {
        let hex_hash = hex_encode(&file.hash);

        let is_new = seen_hashes.insert(hex_hash.clone());
        if is_new {
            let object_path = format!("{OBJECTS_DIR}/{hex_hash}");
            builder
                .append_path_with_name(&file.abs_path, &object_path)
                .with_context(|| format!("failed to archive {}", file.abs_path.display()))?;
        }
        on_file(&file.rel_path, file.size, !is_new);

        manifest.files.push(FileEntry {
            path: file.rel_path,
            hash: hex_hash,
            mode: file.mode,
        });
    }

    let manifest_bytes =
        serde_json::to_vec_pretty(&manifest).context("failed to serialize dedup manifest")?;
    let mut header = tar::Header::new_gnu();
    header.set_size(manifest_bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, MANIFEST_ENTRY, manifest_bytes.as_slice())
        .context("failed to write dedup manifest into archive")?;

    Ok(())
}

/// Unpack an already-decompressed tar payload (read from `reader`) into
/// `dest_dir`. When `dedup` is set, the tar is a dedup bundle: it's first
/// unpacked into `staging_dir`, then resolved into real files at
/// `dest_dir` via [`resolve_dedup_tree`]; `staging_dir` is removed either
/// way once resolution is complete. Shared by `vaqum decompress` and
/// `vaqum diff`, so there is exactly one place that knows how to turn a
/// tar stream back into a directory tree.
pub fn unpack_tar<R: Read>(
    reader: R,
    dedup: bool,
    staging_dir: &Path,
    dest_dir: &Path,
) -> Result<()> {
    if dedup {
        fs::create_dir_all(staging_dir)
            .with_context(|| format!("failed to create {}", staging_dir.display()))?;
        tar::Archive::new(reader)
            .unpack(staging_dir)
            .context("failed to unpack dedup archive")?;
        resolve_dedup_tree(staging_dir, dest_dir)
            .context("failed to reconstruct deduplicated files")?;
        fs::remove_dir_all(staging_dir).ok();
    } else {
        fs::create_dir_all(dest_dir)
            .with_context(|| format!("failed to create {}", dest_dir.display()))?;
        tar::Archive::new(reader)
            .unpack(dest_dir)
            .with_context(|| format!("failed to unpack archive into {}", dest_dir.display()))?;
    }
    Ok(())
}

/// Reconstruct the original directory tree from a dedup archive that has
/// already been unpacked into `staging_dir`, placing the result at
/// `dest_dir`. `staging_dir` is left behind for the caller to clean up.
pub fn resolve_dedup_tree(staging_dir: &Path, dest_dir: &Path) -> Result<()> {
    let manifest_path = staging_dir.join(MANIFEST_ENTRY);
    let manifest_bytes = fs::read(&manifest_path)
        .with_context(|| format!("missing dedup manifest at {}", manifest_path.display()))?;
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).context("failed to parse dedup manifest")?;

    fs::create_dir_all(dest_dir)?;
    for dir in &manifest.dirs {
        fs::create_dir_all(dest_dir.join(dir))
            .with_context(|| format!("failed to create directory {dir}"))?;
    }

    for entry in &manifest.files {
        let object_path = staging_dir.join(OBJECTS_DIR).join(&entry.hash);
        let dest_path = dest_dir.join(&entry.path);
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&object_path, &dest_path).with_context(|| {
            format!(
                "failed to restore {} from object {}",
                entry.path, entry.hash
            )
        })?;
        set_unix_mode(&dest_path, entry.mode)?;
    }

    Ok(())
}
