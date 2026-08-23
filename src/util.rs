#[cfg(unix)]
use std::fs;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use humansize::{DECIMAL, format_size};

/// Pretty-print a byte count the way the CLI examples in the project brief
/// do (e.g. "2.3 GB").
pub fn human_bytes(bytes: u64) -> String {
    format_size(bytes, DECIMAL)
}

/// Stream a file and compute its SHA-256 digest and length without loading
/// it entirely into memory.
pub fn hash_file(path: &Path) -> Result<(u64, [u8; 32])> {
    let mut file =
        File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        sha2::Digest::update(&mut hasher, &buf[..n]);
        total += n as u64;
    }
    let digest = sha2::Digest::finalize(hasher);
    Ok((total, digest.into()))
}

/// One hashed regular file discovered while walking a directory tree.
pub struct TreeFile {
    /// Path relative to the tree root, posix separators.
    pub rel_path: String,
    pub abs_path: PathBuf,
    pub size: u64,
    pub hash: [u8; 32],
    pub mode: Option<u32>,
}

/// Walk `root` and hash every regular file in it, in parallel across
/// `threads` workers. Returns the directory list (including empty
/// directories, relative paths) and the hashed file list, both in a
/// stable, name-sorted order.
///
/// Shared by `compress --dedup`, `dedupe`, and `diff`, so a directory only
/// ever gets walked and hashed one way in this codebase.
///
/// Symlinks and other special files are skipped.
pub fn hash_tree(root: &Path, threads: usize) -> Result<(Vec<String>, Vec<TreeFile>)> {
    let mut dirs = Vec::new();
    let mut candidates: Vec<(String, PathBuf, Option<u32>)> = Vec::new();

    for entry in walkdir::WalkDir::new(root).min_depth(1).sort_by_file_name() {
        let entry = entry.with_context(|| format!("failed to walk {}", root.display()))?;
        let rel = entry
            .path()
            .strip_prefix(root)
            .expect("walkdir entries are under root");
        let rel_str = to_posix_path(rel);

        if entry.file_type().is_dir() {
            dirs.push(rel_str);
        } else if entry.file_type().is_file() {
            candidates.push((rel_str, entry.path().to_path_buf(), unix_mode(entry.path())));
        }
    }

    let pool = build_thread_pool(Some(threads))?;
    let hashes: Vec<Result<(u64, [u8; 32])>> = pool.install(|| {
        use rayon::prelude::*;
        candidates
            .par_iter()
            .map(|(_, abs, _)| hash_file(abs))
            .collect()
    });

    let mut files = Vec::with_capacity(candidates.len());
    for ((rel_path, abs_path, mode), hash_result) in candidates.into_iter().zip(hashes) {
        let (size, hash) =
            hash_result.with_context(|| format!("failed to hash {}", abs_path.display()))?;
        files.push(TreeFile {
            rel_path,
            abs_path,
            size,
            hash,
            mode,
        });
    }

    Ok((dirs, files))
}

/// Unix permission bits for a path, when available on this platform.
#[cfg(unix)]
pub fn unix_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).ok().map(|m| m.permissions().mode())
}

#[cfg(not(unix))]
pub fn unix_mode(_path: &Path) -> Option<u32> {
    None
}

/// Apply previously-captured Unix permission bits to a path, a no-op
/// elsewhere.
#[cfg(unix)]
pub fn set_unix_mode(path: &Path, mode: Option<u32>) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(mode) = mode {
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .with_context(|| format!("failed to set permissions on {}", path.display()))?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn set_unix_mode(_path: &Path, _mode: Option<u32>) -> Result<()> {
    Ok(())
}

/// SHA-256 of an in-memory buffer.
pub fn hash_bytes(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
    sha2::Digest::update(&mut hasher, bytes);
    sha2::Digest::finalize(hasher).into()
}

/// Configure a rayon thread pool sized per `-t/--threads`, or fall back to
/// rayon's default (all cores) when unset.
pub fn build_thread_pool(threads: Option<usize>) -> Result<rayon::ThreadPool> {
    let mut builder = rayon::ThreadPoolBuilder::new();
    if let Some(n) = threads {
        builder = builder.num_threads(n.max(1));
    }
    builder.build().context("failed to set up thread pool")
}

/// Hex-encode a byte slice (e.g. a SHA-256 digest) in lowercase.
pub fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Render a path's components joined with `/`, regardless of platform, so
/// archive entries stay portable between Windows and Unix.
pub fn to_posix_path(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Read a single line of interactive confirmation input from stdin.
pub fn prompt_line(prompt: &str) -> Result<String> {
    use std::io::Write;
    print!("{prompt}");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}
