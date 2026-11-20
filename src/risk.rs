//! Spotting risky things inside an archive: programs dressed up as
//! documents, names that use invisible characters to fake their extension,
//! setuid bits, links pointing out of the archive, archives nested inside
//! archives.
//!
//! This isn't a virus scanner (it can't tell a harmful program from a
//! harmless one). It flags the tricks attackers use to get someone to
//! open a file they shouldn't, so the user can take a closer look.

use serde::Serialize;

/// One flagged entry: where it is and why.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    pub path: String,
    pub reason: String,
}

pub enum Kind<'a> {
    File,
    Dir,
    Symlink(&'a str),
    Hardlink(&'a str),
}

/// Bytes to read from the start of a file for [`assess`].
pub const HEAD_LEN: usize = 8;

/// Opening one of these runs code (on Windows, mostly) without it looking
/// like a program.
const RUNS_CODE: &[&str] = &[
    "scr", "pif", "com", "bat", "cmd", "vbs", "vbe", "js", "jse", "wsf", "wsh", "hta", "ps1",
    "lnk", "msi", "jar", "reg", "cpl", "scf", "inf",
];

/// Programs in their usual, honest form.
const PROGRAMS: &[&str] = &[
    "exe", "app", "dmg", "pkg", "bin", "sh", "command", "elf", "run",
];

/// Extensions people treat as "just a document".
const DOCUMENTS: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "txt", "csv",
    "md", "html", "htm", "jpg", "jpeg", "png", "gif", "bmp", "heic", "webp", "svg", "tif", "tiff",
    "mp3", "mp4", "m4a", "mov", "avi", "mkv", "wav",
];

/// Characters that change how text is displayed without being visible,
/// e.g. a right-to-left override (U+202E) makes `invoice[U+202E]fdp.exe`
/// display as `invoiceexe.pdf`.
fn is_invisible_trick(c: char) -> bool {
    matches!(c,
        '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}'
        | '\u{200B}'..='\u{200D}' | '\u{FEFF}')
}

fn extensions(name: &str) -> Vec<String> {
    let file_name = name.rsplit('/').next().unwrap_or(name);
    file_name
        .split('.')
        .skip(1)
        .map(|e| e.trim().to_ascii_lowercase())
        .collect()
}

/// What the first bytes of a file say it really is.
fn sniff(head: &[u8]) -> Option<&'static str> {
    match head {
        [0x7F, b'E', b'L', b'F', ..] => Some("a Linux program"),
        [0xFE, 0xED, 0xFA, 0xCE | 0xCF, ..]
        | [0xCE | 0xCF, 0xFA, 0xED, 0xFE, ..]
        | [0xCA, 0xFE, 0xBA, 0xBE, ..] => Some("a macOS program"),
        [b'M', b'Z', ..] => Some("a Windows program"),
        [b'#', b'!', ..] => Some("a script"),
        _ => None,
    }
}

fn is_archive(head: &[u8]) -> bool {
    matches!(
        head,
        [b'P', b'K', 3, 4, ..]
            | [0x1F, 0x8B, ..]
            | [b'V', b'A', b'Q', b'M', ..]
            | [b'7', b'z', 0xBC, 0xAF, ..]
            | [b'R', b'a', b'r', b'!', ..]
            | [0xFD, b'7', b'z', b'X', b'Z', ..]
            | [0x28, 0xB5, 0x2F, 0xFD, ..]
    )
}

fn escapes(target: &str) -> bool {
    let t = target.replace('\\', "/");
    t.starts_with('/') || t.split('/').any(|c| c == "..") || t.chars().nth(1) == Some(':')
}

/// Reasons `path` deserves a closer look (empty if none). `head` is the
/// first [`HEAD_LEN`] bytes of a file's contents, when available.
pub fn assess(path: &str, kind: Kind, mode: Option<u32>, head: &[u8]) -> Vec<String> {
    let mut reasons = Vec::new();
    let name = path.rsplit('/').next().unwrap_or(path);

    if name.chars().any(is_invisible_trick) {
        reasons.push(
            "its name contains invisible characters that can disguise its real extension".into(),
        );
    } else if name.chars().any(char::is_control) {
        reasons.push("its name contains control characters".into());
    }

    match kind {
        Kind::File => {
            let exts = extensions(name);
            let last = exts.last().map(String::as_str);
            let before_last = exts.len().checked_sub(2).map(|i| exts[i].as_str());
            let looks_like_doc = last.is_some_and(|e| DOCUMENTS.contains(&e));

            match (before_last, last) {
                (Some(doc), Some(run))
                    if DOCUMENTS.contains(&doc)
                        && (RUNS_CODE.contains(&run) || PROGRAMS.contains(&run)) =>
                {
                    reasons.push(format!("looks like a .{doc} but is really a .{run}"));
                }
                (_, Some(run)) if RUNS_CODE.contains(&run) => {
                    reasons.push(format!("runs code when opened (.{run})"));
                }
                _ => {}
            }
            if looks_like_doc && let Some(what) = sniff(head) {
                reasons.push(format!(
                    "is named like a .{} but is actually {what}",
                    last.unwrap_or_default()
                ));
            }
            if is_archive(head) {
                reasons.push(
                    "is another archive inside this one (its contents weren't checked)".into(),
                );
            }
            if mode.is_some_and(|m| m & 0o6000 != 0) {
                reasons.push(
                    "asks for setuid/setgid permission (run as its owner); vaqum doesn't apply it"
                        .into(),
                );
            }
        }
        Kind::Symlink(target) if escapes(target) => {
            reasons.push(format!(
                "is a link pointing outside the archive (to {target})"
            ));
        }
        Kind::Hardlink(target) if escapes(target) => {
            reasons.push(format!(
                "is a hard link to a path outside the archive ({target})"
            ));
        }
        _ => {}
    }
    reasons
}

/// Prints warnings as a block under a heading (to stderr).
pub fn print(warnings: &[Warning]) {
    if warnings.is_empty() {
        return;
    }
    let n = warnings.len();
    eprintln!();
    eprintln!(
        "⚠  {n} item{} worth a closer look before you open {}:",
        if n == 1 { "" } else { "s" },
        if n == 1 { "it" } else { "them" }
    );
    for w in warnings {
        eprintln!("   {} — {}", w.path, w.reason);
    }
}

pub fn push(warnings: &mut Vec<Warning>, path: &str, reasons: Vec<String>) {
    warnings.extend(reasons.into_iter().map(|reason| Warning {
        path: path.to_string(),
        reason,
    }));
}
