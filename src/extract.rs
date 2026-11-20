//! The one place tar streams are unpacked to disk (plain directory
//! archives, bundles, `--only`, tar.gz). Each entry is unpacked through
//! the tar crate's `unpack_in` (which refuses paths that escape the
//! destination), checked for risky content, and recorded, so callers can
//! carry the download mark over and show warnings afterwards.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::archive::normalize;
use crate::risk::{self, Warning};
use crate::util::to_posix_path;

/// What an extraction wrote, and what looked risky.
#[derive(Default)]
pub struct Extracted {
    /// Every file, directory and link created, in order.
    pub written: Vec<PathBuf>,
    pub warnings: Vec<Warning>,
    /// Number of regular files written.
    pub files: u64,
}

impl Extracted {
    /// Records a regular file written to `path` (shown as `shown`).
    pub fn add_file(&mut self, path: &Path, shown: &str, mode: Option<u32>) {
        let head = read_head(path);
        risk::push(
            &mut self.warnings,
            shown,
            risk::assess(shown, risk::Kind::File, mode, &head),
        );
        self.written.push(path.to_path_buf());
        self.files += 1;
    }

    pub fn add_dir(&mut self, path: &Path) {
        self.written.push(path.to_path_buf());
    }
}

/// First bytes of a file, for content sniffing (empty if unreadable).
pub fn read_head(path: &Path) -> Vec<u8> {
    let mut head = Vec::with_capacity(risk::HEAD_LEN);
    if let Ok(f) = File::open(path) {
        let _ = f.take(risk::HEAD_LEN as u64).read_to_end(&mut head);
    }
    head
}

/// Unpacks the entries of `reader` for which `select(path)` is true into
/// `dest`. `shown_prefix` is prepended to paths in warnings (a directory
/// archive's name).
pub fn unpack_tar<R: Read>(
    reader: R,
    dest: &Path,
    shown_prefix: &str,
    select: &mut dyn FnMut(&str) -> bool,
    out: &mut Extracted,
) -> Result<()> {
    fs::create_dir_all(dest).with_context(|| format!("failed to create {}", dest.display()))?;
    let mut archive = tar::Archive::new(reader);
    for entry in archive
        .entries()
        .context("failed to read archive contents")?
    {
        let mut entry = entry.context("failed to read an archive entry")?;
        let rel = normalize(&to_posix_path(
            &entry
                .path()
                .context("archive entry has an unreadable path")?,
        ));
        let etype = entry.header().entry_type();
        let supported =
            etype.is_file() || etype.is_dir() || etype.is_symlink() || etype.is_hard_link();
        if rel.is_empty() || !supported || !select(&rel) {
            continue;
        }
        let mode = entry.header().mode().ok();
        let link = entry.link_name().ok().flatten().map(|l| to_posix_path(&l));

        if !entry
            .unpack_in(dest)
            .with_context(|| format!("failed to extract {rel}"))?
        {
            bail!("refusing to extract '{rel}': it points outside the output directory");
        }

        let path = dest.join(&rel);
        let shown = format!("{shown_prefix}{rel}");
        if etype.is_file() {
            out.add_file(&path, &shown, mode);
        } else {
            let target = link.as_deref().unwrap_or("");
            let kind = if etype.is_symlink() {
                risk::Kind::Symlink(target)
            } else if etype.is_hard_link() {
                risk::Kind::Hardlink(target)
            } else {
                risk::Kind::Dir
            };
            risk::push(
                &mut out.warnings,
                &shown,
                risk::assess(&shown, kind, mode, &[]),
            );
            out.written.push(path);
        }
    }
    Ok(())
}
