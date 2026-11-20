//! Carrying the "downloaded from the internet" mark from an archive to the
//! files extracted from it.
//!
//! Browsers mark downloads (`com.apple.quarantine` on macOS, the
//! `Zone.Identifier` stream, a.k.a. Mark of the Web, on Windows). That mark
//! is what makes Gatekeeper and SmartScreen check a program before it
//! first runs. If an extractor drops it, a malicious program pulled out of
//! a downloaded archive skips those checks, so vaqum copies the mark onto
//! everything it extracts, as macOS Archive Utility and Windows Explorer do.

use std::path::{Path, PathBuf};

/// The archive's download mark, if it has one.
pub struct Origin {
    #[allow(dead_code)]
    data: Vec<u8>,
}

#[cfg(target_os = "macos")]
const QUARANTINE: &str = "com.apple.quarantine";

impl Origin {
    /// Reads the mark from `archive`. `None` if it has none (or on
    /// platforms without one).
    pub fn of(archive: &Path) -> Option<Origin> {
        #[cfg(target_os = "macos")]
        {
            xattr::get(archive, QUARANTINE)
                .ok()
                .flatten()
                .map(|data| Origin { data })
        }
        #[cfg(windows)]
        {
            std::fs::read(zone_stream(archive))
                .ok()
                .map(|data| Origin { data })
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = archive;
            None
        }
    }

    /// Copies the mark onto each extracted path. Best-effort: a filesystem
    /// that can't store it isn't an error.
    pub fn apply(&self, paths: &[PathBuf]) {
        for path in paths {
            #[cfg(target_os = "macos")]
            {
                let _ = xattr::set(path, QUARANTINE, &self.data);
            }
            #[cfg(windows)]
            {
                if path.is_file() {
                    let _ = std::fs::write(zone_stream(path), &self.data);
                }
            }
            #[cfg(not(any(target_os = "macos", windows)))]
            {
                let _ = path;
            }
        }
    }
}

#[cfg(windows)]
fn zone_stream(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(":Zone.Identifier");
    PathBuf::from(s)
}
