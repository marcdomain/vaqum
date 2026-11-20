//! Reading what's inside an archive: opening a `.vaqum` payload stream
//! (from a file or stdin), walking its entries for `vaqum list`, and
//! matching entry paths against `decompress --only` selectors.
//!
//! Paths are shown and matched the way they land on disk after a full
//! `decompress`: a directory archive's entries sit under the archive's
//! name (`backup/docs/a.txt`), a bundle's entries sit at the top level.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::codec;
use crate::crypto;
use crate::dedup::{MANIFEST_ENTRY, Manifest, OBJECTS_DIR};
use crate::format::{EntryType, Header, SizeLimitReader};
use crate::util::{is_stdio, to_posix_path};

/// An open `.vaqum` archive, header parsed, positioned at the payload.
pub struct VaqumSource {
    pub header: Header,
    pub reader: Box<dyn Read>,
    /// Payload bytes on disk; unknown when reading from stdin.
    pub compressed_size: Option<u64>,
}

pub fn open_vaqum(path: &Path) -> Result<VaqumSource> {
    if is_stdio(path) {
        let mut stdin = io::stdin().lock();
        let header = Header::read(&mut stdin).context("reading a .vqm archive from stdin")?;
        return Ok(VaqumSource {
            header,
            reader: Box::new(stdin),
            compressed_size: None,
        });
    }
    let mut file =
        File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let total = file.metadata()?.len();
    let header = Header::read(&mut file)?;
    let compressed_size = Some(total.saturating_sub(header.on_disk_len()));
    Ok(VaqumSource {
        header,
        reader: Box::new(file),
        compressed_size,
    })
}

/// Decrypt (if needed) and decompress the payload, yielding the original
/// bytes: the file itself, or the tar stream for directory archives and
/// bundles.
pub fn decoded_payload<R: Read>(
    reader: R,
    header: &Header,
    key_file: Option<&Path>,
) -> Result<SizeLimitReader<codec::Decoder<crypto::DecryptReader<R>>>> {
    let key = header
        .encrypted
        .then(|| crypto::key_for_archive(key_file, &header.salt, header.strong_kdf))
        .transpose()?;
    let key_ref = key.as_ref().map(|k| (k, &header.nonce_prefix));
    let decoder = codec::Decoder::new(
        crypto::DecryptReader::new(reader, key_ref),
        header.algorithm,
    )?;
    Ok(SizeLimitReader::new(decoder, header.original_size))
}

/// Joins an archive-supplied relative path onto `dest_dir`, refusing paths
/// that escape it: `..`, absolute paths, or a parent directory that is
/// (or sits under) a symlink pointing outside `dest_dir`. Creates the
/// parent directories.
pub fn safe_join(dest_dir: &Path, rel: &str) -> Result<std::path::PathBuf> {
    if !is_safe_relative(rel) {
        bail!("refusing to extract '{rel}': it points outside the output directory");
    }
    let dest = dest_dir.join(rel);
    let parent = dest.parent().unwrap_or(dest_dir);
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;
    let root = dest_dir
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", dest_dir.display()))?;
    let real_parent = parent
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", parent.display()))?;
    if !real_parent.starts_with(&root) {
        bail!(
            "refusing to extract '{rel}': it would be written through a link outside the output directory"
        );
    }
    // Never write through an existing symlink at the destination itself.
    if std::fs::symlink_metadata(&dest).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("refusing to extract '{rel}': a symlink already exists at that path");
    }
    Ok(dest)
}

/// A dedup object key must be a SHA-256 hex digest (it's used as a path).
pub fn is_object_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

// ------------------------------------------------------------------ paths

/// Normalizes a user- or archive-supplied path: `/` separators, no leading
/// `./` or `/`, no trailing `/`.
pub fn normalize(path: &str) -> String {
    let mut s = path.replace('\\', "/");
    loop {
        if let Some(rest) = s.strip_prefix("./") {
            s = rest.to_string();
        } else if let Some(rest) = s.strip_prefix('/') {
            s = rest.to_string();
        } else {
            break;
        }
    }
    s.trim_end_matches('/').to_string()
}

/// Rejects archive paths that would escape the output directory.
pub fn is_safe_relative(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// How an entry's stored (tar-relative) path is shown to the user.
pub fn display_path(header: &Header, rel: &str) -> String {
    match header.entry_type {
        EntryType::Archive => format!("{}/{rel}", header.name),
        _ => rel.to_string(),
    }
}

/// `decompress --only` selectors. A plain path matches that entry, or
/// everything inside it when it's a directory. A pattern with wildcards
/// (`*`, `?`, `[...]`, `{a,b}`) works like `.gitignore`: without a `/` it
/// matches a name at any depth (`*.rs`); with one it matches the whole path
/// (`docs/**/*.md`, where `*` stays within one folder and `**` spans any
/// number). A matching folder brings everything inside it.
pub struct Selectors {
    items: Vec<Selector>,
    matched: Vec<bool>,
}

struct Selector {
    raw: String,
    glob: Option<globset::GlobMatcher>,
    /// Pattern has no `/`: match against each path component instead.
    any_depth: bool,
}

impl Selector {
    fn new(raw: &str) -> Result<Self> {
        let raw = normalize(raw);
        let is_glob = raw.contains(['*', '?', '[', '{']);
        let glob = if is_glob {
            Some(
                globset::GlobBuilder::new(&raw)
                    .literal_separator(true)
                    .build()
                    .with_context(|| format!("invalid --only pattern '{raw}'"))?
                    .compile_matcher(),
            )
        } else {
            None
        };
        Ok(Self {
            any_depth: !raw.contains('/'),
            raw,
            glob,
        })
    }

    fn covers(&self, path: &str) -> bool {
        match &self.glob {
            None => path == self.raw || path.starts_with(&format!("{}/", self.raw)),
            Some(glob) if self.any_depth => path.split('/').any(|c| glob.is_match(c)),
            // The path itself or any folder above it.
            Some(glob) => {
                let mut prefix = String::new();
                path.split('/').any(|c| {
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(c);
                    glob.is_match(&prefix)
                })
            }
        }
    }
}

impl Selectors {
    pub fn new(raw: &[String]) -> Result<Self> {
        let items = raw
            .iter()
            .map(|s| Selector::new(s))
            .collect::<Result<Vec<_>>>()?;
        let matched = vec![false; items.len()];
        Ok(Self { items, matched })
    }

    /// True if `rel` (the stored path) should be extracted. For directory
    /// archives, selectors may include the archive's name prefix or not.
    pub fn select(&mut self, header: &Header, rel: &str) -> bool {
        let rel = normalize(rel);
        let shown = display_path(header, &rel);
        let mut hit = false;
        for (i, sel) in self.items.iter().enumerate() {
            let whole_archive = header.entry_type == EntryType::Archive
                && sel.glob.is_none()
                && sel.raw == header.name;
            if sel.covers(&rel) || sel.covers(&shown) || whole_archive {
                self.matched[i] = true;
                hit = true;
            }
        }
        hit
    }

    /// Fails if any selector matched nothing.
    pub fn ensure_all_matched(&self) -> Result<()> {
        let missing: Vec<&str> = self
            .items
            .iter()
            .zip(&self.matched)
            .filter(|(_, m)| !**m)
            .map(|(s, _)| s.raw.as_str())
            .collect();
        if !missing.is_empty() {
            bail!(
                "nothing in the archive matches: {} (run `vaqum list <archive>` to see its paths)",
                missing.join(", ")
            );
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- listing

#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    File,
    Dir,
    Link,
}

#[derive(Serialize)]
pub struct ListEntry {
    pub path: String,
    pub kind: Kind,
    pub size: u64,
    /// Where a link points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Reasons this entry deserves a closer look (see `risk.rs`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl ListEntry {
    pub fn dir(path: String) -> Self {
        let warnings = crate::risk::assess(&path, crate::risk::Kind::Dir, None, &[]);
        Self {
            path,
            kind: Kind::Dir,
            size: 0,
            target: None,
            warnings,
        }
    }

    pub fn file(path: String, size: u64, mode: Option<u32>, head: &[u8]) -> Self {
        let warnings = crate::risk::assess(&path, crate::risk::Kind::File, mode, head);
        Self {
            path,
            kind: Kind::File,
            size,
            target: None,
            warnings,
        }
    }

    pub fn link(path: String, target: String, hard: bool) -> Self {
        let kind = if hard {
            crate::risk::Kind::Hardlink(&target)
        } else {
            crate::risk::Kind::Symlink(&target)
        };
        let warnings = crate::risk::assess(&path, kind, None, &[]);
        Self {
            path,
            kind: Kind::Link,
            size: 0,
            target: Some(target),
            warnings,
        }
    }
}

/// Reads the first bytes of an entry's contents, for content sniffing.
pub fn read_head<R: Read>(r: R) -> Vec<u8> {
    let mut head = Vec::with_capacity(crate::risk::HEAD_LEN);
    let _ = r.take(crate::risk::HEAD_LEN as u64).read_to_end(&mut head);
    head
}

/// Turns one tar entry into a listing row (`None` for entry types that
/// aren't files, folders or links).
pub fn tar_list_entry<R: Read>(entry: &mut tar::Entry<R>, shown: String) -> Option<ListEntry> {
    let etype = entry.header().entry_type();
    let mode = entry.header().mode().ok();
    if etype.is_dir() {
        Some(ListEntry::dir(shown))
    } else if etype.is_file() {
        let size = entry.size();
        let head = read_head(&mut *entry);
        Some(ListEntry::file(shown, size, mode, &head))
    } else if etype.is_symlink() || etype.is_hard_link() {
        let target = entry
            .link_name()
            .ok()
            .flatten()
            .map(|l| to_posix_path(&l))
            .unwrap_or_default();
        Some(ListEntry::link(shown, target, etype.is_hard_link()))
    } else {
        None
    }
}

fn tar_entry_path<R: Read>(entry: &tar::Entry<R>) -> Result<String> {
    let path = entry
        .path()
        .context("archive entry has an unreadable path")?;
    Ok(normalize(&to_posix_path(&path)))
}

/// Every file and directory in a decoded `.vaqum` payload, in archive order.
pub fn list_vaqum<R: Read>(mut payload: R, header: &Header) -> Result<Vec<ListEntry>> {
    if header.entry_type == EntryType::File {
        let head = read_head(&mut payload);
        return Ok(vec![ListEntry::file(
            header.name.clone(),
            header.original_size,
            None,
            &head,
        )]);
    }

    let mut archive = tar::Archive::new(payload);
    let entries = archive
        .entries()
        .context("failed to read archive contents")?;

    if !header.dedup {
        let mut out = Vec::new();
        for entry in entries {
            let mut entry = entry.context("failed to read an archive entry")?;
            let rel = tar_entry_path(&entry)?;
            if rel.is_empty() {
                continue;
            }
            if let Some(row) = tar_list_entry(&mut entry, display_path(header, &rel)) {
                out.push(row);
            }
        }
        return Ok(out);
    }

    // Dedup archives store content objects first and the manifest (the real
    // paths) last, so collect object sizes and first bytes on the way through.
    let mut objects: HashMap<String, (u64, Vec<u8>)> = HashMap::new();
    let mut manifest: Option<Manifest> = None;
    for entry in entries {
        let mut entry = entry.context("failed to read an archive entry")?;
        let rel = tar_entry_path(&entry)?;
        if rel == MANIFEST_ENTRY {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            manifest =
                Some(serde_json::from_slice(&bytes).context("failed to parse dedup manifest")?);
        } else if let Some(hash) = rel.strip_prefix(&format!("{OBJECTS_DIR}/")) {
            let size = entry.size();
            objects.insert(hash.to_string(), (size, read_head(&mut entry)));
        }
    }
    let manifest = manifest.context("dedup archive is missing its manifest")?;
    let mut out: Vec<ListEntry> = manifest
        .dirs
        .iter()
        .map(|d| ListEntry::dir(display_path(header, &normalize(d))))
        .collect();
    for f in &manifest.files {
        let (size, head) = objects.get(&f.hash).cloned().unwrap_or_default();
        out.push(ListEntry::file(
            display_path(header, &normalize(&f.path)),
            size,
            f.mode,
            &head,
        ));
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}
