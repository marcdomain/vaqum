use std::fs;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use humansize::{DECIMAL, format_size};
use indicatif::ProgressBar;

use crate::exclude::ExcludeSet;

/// Pretty-print a byte count the way the CLI examples in the project brief
/// do (e.g. "2.3 GB").
pub fn human_bytes(bytes: u64) -> String {
    format_size(bytes, DECIMAL)
}

/// Format a filesystem timestamp as RFC 3339 in UTC (avoids the local
/// timezone lookup, which is unsound to call in a multithreaded process on
/// some Unix platforms).
pub fn format_time(t: std::time::SystemTime) -> String {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    OffsetDateTime::from(t)
        .replace_nanosecond(0)
        .unwrap_or_else(|_| OffsetDateTime::from(t))
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Stream a file and compute its SHA-256 digest and length without loading
/// it entirely into memory.
pub fn hash_file(path: &Path) -> Result<(u64, [u8; 32])> {
    hash_file_with_progress(path, &ProgressBar::hidden())
}

/// [`hash_file`], advancing `bar` as it reads.
pub fn hash_file_with_progress(path: &Path, bar: &ProgressBar) -> Result<(u64, [u8; 32])> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut file = crate::progress::ProgressReader::new(file, bar.clone());
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
/// ever gets walked and hashed one way in this codebase. `exclude`, when
/// given, prunes matching entries (and doesn't descend into excluded
/// directories at all). `progress`, when given, is advanced by each file's
/// size as it finishes hashing.
///
/// Symlinks and other special files are skipped.
pub fn hash_tree(
    root: &Path,
    threads: usize,
    exclude: Option<&ExcludeSet>,
    progress: Option<&ProgressBar>,
) -> Result<(Vec<String>, Vec<TreeFile>)> {
    let mut dirs = Vec::new();
    let mut candidates: Vec<(String, PathBuf, Option<u32>)> = Vec::new();

    let walker = walkdir::WalkDir::new(root)
        .min_depth(1)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| !exclude.is_some_and(|ex| ex.excludes(root, e)));

    for entry in walker {
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
            .map(|(_, abs, _)| {
                let result = hash_file(abs);
                if let (Some(bar), Ok((size, _))) = (progress, &result) {
                    bar.inc(*size);
                }
                result
            })
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

/// Heuristic: valid UTF-8 with no NUL byte in the first 8KB looks like
/// text. Shared by `diff` (to decide whether to line-diff or report
/// "binary files differ") and `search` (to skip binary files when
/// grepping content).
pub fn as_text(bytes: &[u8]) -> Option<&str> {
    let probe_len = bytes.len().min(8000);
    if bytes[..probe_len].contains(&0) {
        return None;
    }
    std::str::from_utf8(bytes).ok()
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
    let mut out = String::new();
    for c in path.components() {
        if let std::path::Component::RootDir = c {
            out.push('/');
            continue;
        }
        if !out.is_empty() && !out.ends_with('/') {
            out.push('/');
        }
        out.push_str(&c.as_os_str().to_string_lossy());
    }
    out
}

/// `-` as a path means stdin (for an input) or stdout (for an output).
pub fn is_stdio(path: &Path) -> bool {
    path.as_os_str() == "-"
}

/// Status messages normally go to stdout, but when stdout is carrying the
/// archive/file data itself they must go to stderr instead.
#[derive(Clone, Copy)]
pub struct Out {
    to_stderr: bool,
}

impl Out {
    pub fn new(to_stderr: bool) -> Self {
        Self { to_stderr }
    }

    pub fn say(&self, msg: impl std::fmt::Display) {
        if self.to_stderr {
            eprintln!("{msg}");
        } else {
            println!("{msg}");
        }
    }
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

/// Temp files being written right now, so an interrupt (Ctrl-C) can
/// remove them instead of leaving half-written files behind.
static IN_PROGRESS: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

fn track(path: &Path) {
    IN_PROGRESS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(path.to_path_buf());
}

fn untrack(path: &Path) {
    IN_PROGRESS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|p| p != path);
}

/// On Ctrl-C: delete in-progress temp files, then exit with the usual
/// interrupted status (130). Call once at startup.
pub fn install_interrupt_cleanup() {
    let _ = ctrlc::set_handler(|| {
        for path in IN_PROGRESS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
        {
            let _ = fs::remove_file(path);
        }
        eprintln!("\ninterrupted; nothing half-written was left behind");
        std::process::exit(130);
    });
}

/// Writes a file safely: data goes to a new, randomly named temp file next
/// to `dest` (created exclusively, so a pre-planted file or symlink can't
/// hijack it), which is renamed over `dest` only on [`AtomicFile::commit`].
/// If anything fails first, the temp file is removed and `dest` is left
/// untouched, so a failed run never leaves a half-written output.
pub struct AtomicFile {
    tmp: PathBuf,
    dest: PathBuf,
    file: Option<File>,
}

impl AtomicFile {
    pub fn create(dest: &Path) -> Result<Self> {
        let dir = match dest.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let base = dest
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "vaqum".to_string());
        for _ in 0..16 {
            let mut suffix = [0u8; 6];
            rand::Rng::fill_bytes(&mut rand::rng(), &mut suffix);
            let tmp = dir.join(format!(".{base}.{}.tmp", hex_encode(&suffix)));
            match fs::OpenOptions::new()
                .write(true)
                .read(true)
                .create_new(true)
                .open(&tmp)
            {
                Ok(file) => {
                    track(&tmp);
                    return Ok(Self {
                        tmp,
                        dest: dest.to_path_buf(),
                        file: Some(file),
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!("failed to create a temporary file in {}", dir.display())
                    });
                }
            }
        }
        anyhow::bail!(
            "could not create a unique temporary file in {}",
            dir.display()
        )
    }

    /// The temp file's path (until it's committed).
    pub fn temp_path(&self) -> &Path {
        &self.tmp
    }

    pub fn file(&mut self) -> &mut File {
        self.file.as_mut().expect("file is open until commit")
    }

    /// Moves the finished file into place.
    pub fn commit(mut self) -> Result<()> {
        if let Some(file) = self.file.take() {
            use std::io::Write;
            let mut file = file;
            file.flush()?;
            // Durable before it becomes visible: a crash after the rename
            // must never leave a truncated file under the final name.
            file.sync_all()
                .with_context(|| format!("failed to write {}", self.dest.display()))?;
        }
        fs::rename(&self.tmp, &self.dest)
            .with_context(|| format!("failed to write {}", self.dest.display()))?;
        untrack(&self.tmp);
        self.tmp = PathBuf::new();
        Ok(())
    }
}

impl Drop for AtomicFile {
    fn drop(&mut self) {
        self.file.take();
        if !self.tmp.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.tmp);
            untrack(&self.tmp);
        }
    }
}
