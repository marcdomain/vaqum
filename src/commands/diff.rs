//! `vaqum diff` — compare two files, directories, or `.vaqum` archives (any
//! mix of the three; `.vaqum` inputs are transparently decompressed first).
//!
//! Exit-code contract mirrors the classic Unix `diff`: 0 identical, 1
//! differences found, 2 trouble. See `main.rs` for where that's enforced.

use std::fs::{self, File};
use std::io::{IsTerminal, Read, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use similar::{ChangeTag, TextDiff};
use tempfile::TempDir;

use crate::cli::DiffArgs;
use crate::codec;
use crate::dedup;
use crate::format::{EntryType, Header};
use crate::util::{TreeFile, as_text, hash_bytes, hash_tree, human_bytes};

pub fn run(args: DiffArgs) -> Result<bool> {
    let a = resolve(&args.a)?;
    let b = resolve(&args.b)?;

    let (identical, html_body, tree_for_editor) = match (&a, &b) {
        (
            Resolved::File {
                name: na,
                bytes: ba,
            },
            Resolved::File {
                name: nb,
                bytes: bb,
            },
        ) => {
            let identical = ba == bb;
            print_file_diff(na, ba, nb, bb, identical);
            let body = html_file_diff(na, ba, nb, bb, identical);
            (identical, body, None)
        }
        (
            Resolved::Dir {
                name: na, root: ra, ..
            },
            Resolved::Dir {
                name: nb, root: rb, ..
            },
        ) => {
            let threads = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1);
            let tree = compute_tree_diff(ra, rb, threads)?;
            print_tree_diff(na, nb, &tree, args.verbose);
            let body = html_tree_diff(na, nb, &tree);
            let identical = tree.is_identical();
            (identical, body, Some(tree))
        }
        _ => bail!(
            "cannot compare a file against a directory: '{}' is a {}, '{}' is a {}",
            args.a.display(),
            a.kind_name(),
            args.b.display(),
            b.kind_name(),
        ),
    };

    if args.html.is_some() || args.open {
        let html_path = match &args.html {
            Some(p) => p.clone(),
            None => {
                let tmp = tempfile::Builder::new()
                    .prefix("vaqum-diff-")
                    .suffix(".html")
                    .tempfile()
                    .context("failed to create a temp file for --open")?;
                let (_, path) = tmp.keep().context("failed to persist temp html file")?;
                path
            }
        };
        let page = html_page(&args.a, &args.b, &html_body);
        fs::write(&html_path, page)
            .with_context(|| format!("failed to write {}", html_path.display()))?;
        println!("\nHTML diff report: {}", html_path.display());
        if args.open {
            open_in_browser(&html_path);
        }
    }

    if args.editor {
        open_in_editor(&args, &a, &b, identical, tree_for_editor.as_ref())?;
    }

    Ok(identical)
}

// ---------------------------------------------------------------------
// Editor mode (`--editor`) — hands off to VS Code's (or another editor's)
// live, editable `--diff` view instead of a static report.
// ---------------------------------------------------------------------

fn open_in_editor(
    args: &DiffArgs,
    a: &Resolved,
    b: &Resolved,
    identical: bool,
    tree: Option<&TreeDiff>,
) -> Result<()> {
    let editor = std::env::var("VAQUM_DIFF_EDITOR").unwrap_or_else(|_| "code".to_string());

    match (a, b, tree) {
        (Resolved::File { name: na, .. }, Resolved::File { name: nb, .. }, None) => {
            if identical {
                println!("\n(no editor diff opened — files are identical)");
                return Ok(());
            }
            let (path_a, from_archive_a) = materialize_for_editor(&args.a, a)?;
            let (path_b, from_archive_b) = materialize_for_editor(&args.b, b)?;
            if from_archive_a || from_archive_b {
                println!(
                    "\nnote: side(s) decompressed from a .vaqum archive are scratch copies — edits there won't be saved back into the archive."
                );
            }
            println!("\nopening '{na}' vs '{nb}' in `{editor} --diff` ...");
            launch_editor_diff(&editor, &path_a, &path_b);
        }
        (Resolved::Dir { root: ra, .. }, Resolved::Dir { root: rb, .. }, Some(tree)) => {
            open_editor_for_tree(&editor, ra, rb, tree);
        }
        _ => unreachable!("run() already rejected file-vs-directory comparisons"),
    }

    Ok(())
}

/// Returns a real on-disk path an editor can open, plus whether it's a
/// throwaway scratch copy (decompressed from a `.vaqum` archive) rather
/// than the user's actual file.
fn materialize_for_editor(original: &Path, resolved: &Resolved) -> Result<(PathBuf, bool)> {
    let from_archive =
        original.is_file() && crate::format::is_vaqum_file(original).unwrap_or(false);
    let path = match resolved {
        Resolved::Dir { root, .. } => root.clone(),
        Resolved::File { bytes, .. } => {
            if from_archive {
                let mut tmp = tempfile::Builder::new()
                    .prefix("vaqum-diff-")
                    .tempfile()
                    .context("failed to create a scratch file for the editor")?;
                tmp.write_all(bytes)?;
                let (_, path) = tmp.keep().context("failed to persist scratch file")?;
                path
            } else {
                original.to_path_buf()
            }
        }
    };
    Ok((path, from_archive))
}

fn open_editor_for_tree(editor: &str, root_a: &Path, root_b: &Path, tree: &TreeDiff) {
    const MAX_TABS: usize = 15;

    let text_modified: Vec<&ModifiedFile> = tree
        .modified
        .iter()
        .filter(|f| matches!(f.content, ModifiedContent::Text { .. }))
        .collect();

    if text_modified.is_empty() {
        println!("\n(no modified text files to open in the editor)");
        return;
    }

    let opening = text_modified.len().min(MAX_TABS);
    println!("\nopening {opening} modified file(s) in `{editor} --diff` ...");
    for f in text_modified.iter().take(MAX_TABS) {
        launch_editor_diff(editor, &root_a.join(&f.rel_path), &root_b.join(&f.rel_path));
    }
    if text_modified.len() > MAX_TABS {
        println!(
            "  ({} more modified file(s) not opened — use --html for the full report)",
            text_modified.len() - MAX_TABS
        );
    }
    let binary_count = tree.modified.len() - text_modified.len();
    if binary_count > 0 {
        println!("  ({binary_count} modified binary file(s) skipped — not diffable as text)");
    }
}

fn launch_editor_diff(editor: &str, path_a: &Path, path_b: &Path) {
    let result = std::process::Command::new(editor)
        .arg("--diff")
        .arg(path_a)
        .arg(path_b)
        .status();
    if let Err(err) = result {
        eprintln!(
            "warning: could not launch `{editor} --diff`: {err}. Is it installed and on PATH? \
(in VS Code: Cmd/Ctrl+Shift+P -> \"Shell Command: Install 'code' command in PATH\"). \
Override the editor with $VAQUM_DIFF_EDITOR."
        );
    }
}

// ---------------------------------------------------------------------
// Resolving inputs (plain file/dir, or a .vaqum archive of either)
// ---------------------------------------------------------------------

enum Resolved {
    File {
        name: String,
        bytes: Vec<u8>,
    },
    Dir {
        name: String,
        root: PathBuf,
        // Kept alive for the duration of the diff when this side came from
        // a .vaqum archive; `None` for a real on-disk directory.
        _temp: Option<TempDir>,
    },
}

impl Resolved {
    fn kind_name(&self) -> &'static str {
        match self {
            Resolved::File { .. } => "file",
            Resolved::Dir { .. } => "directory",
        }
    }
}

fn resolve(path: &Path) -> Result<Resolved> {
    if !path.exists() {
        bail!("'{}' does not exist", path.display());
    }
    if path.is_file() && crate::format::is_vaqum_file(path)? {
        return resolve_vaqum(path);
    }
    if path.is_dir() {
        let name = display_name(path);
        return Ok(Resolved::Dir {
            name,
            root: path.to_path_buf(),
            _temp: None,
        });
    }
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(Resolved::File {
        name: display_name(path),
        bytes,
    })
}

fn resolve_vaqum(path: &Path) -> Result<Resolved> {
    let mut in_file =
        File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let header = Header::read(&mut in_file)?;
    if header.encrypted {
        bail!(
            "'{}' is encrypted; diff doesn't support encrypted archives yet",
            path.display()
        );
    }
    let decoder = codec::Decoder::new(in_file, header.algorithm)?;

    match header.entry_type {
        EntryType::File => {
            let mut bytes = Vec::new();
            let mut decoder = decoder;
            decoder
                .read_to_end(&mut bytes)
                .with_context(|| format!("failed to decompress {}", path.display()))?;
            Ok(Resolved::File {
                name: header.name,
                bytes,
            })
        }
        EntryType::Archive => {
            let temp = TempDir::new().context("failed to create a temp directory")?;
            let staging_dir = temp.path().join(".vaqum-diff-staging");
            let target_dir = temp.path().join(&header.name);
            dedup::unpack_tar(decoder, header.dedup, &staging_dir, &target_dir)?;
            Ok(Resolved::Dir {
                name: header.name,
                root: target_dir,
                _temp: Some(temp),
            })
        }
        EntryType::Bundle => {
            let temp = TempDir::new().context("failed to create a temp directory")?;
            let staging_dir = temp.path().join(".vaqum-diff-staging");
            let target_dir = temp.path().join("bundle");
            dedup::unpack_tar(decoder, header.dedup, &staging_dir, &target_dir)?;
            Ok(Resolved::Dir {
                name: header.name,
                root: target_dir,
                _temp: Some(temp),
            })
        }
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

// ---------------------------------------------------------------------
// Directory tree diffing
// ---------------------------------------------------------------------

struct TreeFileSummary {
    rel_path: String,
    size: u64,
}

enum ModifiedContent {
    Text { text_a: String, text_b: String },
    Binary { size_a: u64, size_b: u64 },
}

struct ModifiedFile {
    rel_path: String,
    content: ModifiedContent,
}

struct TreeDiff {
    added: Vec<TreeFileSummary>,
    removed: Vec<TreeFileSummary>,
    modified: Vec<ModifiedFile>,
    unchanged_count: usize,
}

impl TreeDiff {
    fn is_identical(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }
}

fn compute_tree_diff(root_a: &Path, root_b: &Path, threads: usize) -> Result<TreeDiff> {
    let (_, files_a) = hash_tree(root_a, threads, None, None)?;
    let (_, files_b) = hash_tree(root_b, threads, None, None)?;

    let map_a: std::collections::BTreeMap<&str, &TreeFile> =
        files_a.iter().map(|f| (f.rel_path.as_str(), f)).collect();
    let map_b: std::collections::BTreeMap<&str, &TreeFile> =
        files_b.iter().map(|f| (f.rel_path.as_str(), f)).collect();

    let all_paths: std::collections::BTreeSet<&str> =
        map_a.keys().chain(map_b.keys()).copied().collect();

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();
    let mut unchanged_count = 0;

    for rel_path in all_paths {
        match (map_a.get(rel_path), map_b.get(rel_path)) {
            (Some(fa), Some(fb)) if fa.hash == fb.hash => unchanged_count += 1,
            (Some(fa), Some(fb)) => {
                let content = read_modified_content(fa, fb)?;
                modified.push(ModifiedFile {
                    rel_path: rel_path.to_string(),
                    content,
                });
            }
            (Some(fa), None) => removed.push(TreeFileSummary {
                rel_path: rel_path.to_string(),
                size: fa.size,
            }),
            (None, Some(fb)) => added.push(TreeFileSummary {
                rel_path: rel_path.to_string(),
                size: fb.size,
            }),
            (None, None) => unreachable!("path came from one of the two maps"),
        }
    }

    Ok(TreeDiff {
        added,
        removed,
        modified,
        unchanged_count,
    })
}

fn read_modified_content(fa: &TreeFile, fb: &TreeFile) -> Result<ModifiedContent> {
    let bytes_a = fs::read(&fa.abs_path)
        .with_context(|| format!("failed to read {}", fa.abs_path.display()))?;
    let bytes_b = fs::read(&fb.abs_path)
        .with_context(|| format!("failed to read {}", fb.abs_path.display()))?;
    match (as_text(&bytes_a), as_text(&bytes_b)) {
        (Some(ta), Some(tb)) => Ok(ModifiedContent::Text {
            text_a: ta.to_string(),
            text_b: tb.to_string(),
        }),
        _ => Ok(ModifiedContent::Binary {
            size_a: fa.size,
            size_b: fb.size,
        }),
    }
}

// ---------------------------------------------------------------------
// Terminal rendering
// ---------------------------------------------------------------------

fn print_file_diff(name_a: &str, bytes_a: &[u8], name_b: &str, bytes_b: &[u8], identical: bool) {
    if identical {
        println!("'{name_a}' and '{name_b}' are identical");
        return;
    }
    match (as_text(bytes_a), as_text(bytes_b)) {
        (Some(ta), Some(tb)) => print_unified_diff(name_a, name_b, ta, tb),
        _ => println!(
            "Binary files '{name_a}' and '{name_b}' differ ({} vs {}; sha256 {}… vs {}…)",
            human_bytes(bytes_a.len() as u64),
            human_bytes(bytes_b.len() as u64),
            &crate::util::hex_encode(&hash_bytes(bytes_a))[..12],
            &crate::util::hex_encode(&hash_bytes(bytes_b))[..12],
        ),
    }
}

fn print_tree_diff(name_a: &str, name_b: &str, diff: &TreeDiff, verbose: bool) {
    if diff.is_identical() {
        println!(
            "'{name_a}' and '{name_b}' are identical ({} files)",
            diff.unchanged_count
        );
        return;
    }

    println!("comparing '{name_a}' vs '{name_b}':");
    println!(
        "  {} unchanged, {} added, {} removed, {} modified",
        diff.unchanged_count,
        diff.added.len(),
        diff.removed.len(),
        diff.modified.len()
    );
    for f in &diff.removed {
        println!("  - {}  ({})", f.rel_path, human_bytes(f.size));
    }
    for f in &diff.added {
        println!("  + {}  ({})", f.rel_path, human_bytes(f.size));
    }
    for f in &diff.modified {
        match &f.content {
            ModifiedContent::Binary { size_a, size_b } => println!(
                "  ~ {}  (binary, {} -> {})",
                f.rel_path,
                human_bytes(*size_a),
                human_bytes(*size_b)
            ),
            ModifiedContent::Text { text_a, text_b } => {
                println!("  ~ {}", f.rel_path);
                if verbose {
                    let name_a = format!("a/{}", f.rel_path);
                    let name_b = format!("b/{}", f.rel_path);
                    print_unified_diff(&name_a, &name_b, text_a, text_b);
                }
            }
        }
    }
}

fn print_unified_diff(name_a: &str, name_b: &str, text_a: &str, text_b: &str) {
    let diff_text = TextDiff::from_lines(text_a, text_b)
        .unified_diff()
        .header(name_a, name_b)
        .to_string();
    let colorize = std::io::stdout().is_terminal();
    for line in diff_text.lines() {
        if !colorize {
            println!("{line}");
        } else if line.starts_with("@@") {
            println!("\x1b[36m{line}\x1b[0m");
        } else if (line.starts_with('+') && !line.starts_with("+++"))
            || (line.starts_with('-') && !line.starts_with("---"))
        {
            let color = if line.starts_with('+') { "32" } else { "31" };
            println!("\x1b[{color}m{line}\x1b[0m");
        } else {
            println!("{line}");
        }
    }
}

// ---------------------------------------------------------------------
// HTML rendering — a self-contained, offline diff report with no
// JavaScript: `<details>`/`<summary>` give us collapsible sections for
// free, and `prefers-color-scheme` keeps it readable in either theme.
// ---------------------------------------------------------------------

const STYLE: &str = r#"
:root {
  color-scheme: light dark;
  --bg: #ffffff; --fg: #1a1a1a; --muted: #6b7280; --border: #e5e7eb;
  --add-bg: #e6ffed; --add-fg: #1a7f37; --del-bg: #ffeef0; --del-fg: #cf222e;
  --hunk-bg: #f6f8fa; --card-bg: #f9fafb;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #0d1117; --fg: #e6edf3; --muted: #9198a1; --border: #30363d;
    --add-bg: #033a16; --add-fg: #3fb950; --del-bg: #490202; --del-fg: #f85149;
    --hunk-bg: #161b22; --card-bg: #161b22;
  }
}
* { box-sizing: border-box; }
body {
  background: var(--bg); color: var(--fg); margin: 0; padding: 2rem;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  line-height: 1.5;
}
h1 { font-size: 1.25rem; margin: 0 0 1rem; }
.summary {
  background: var(--card-bg); border: 1px solid var(--border); border-radius: 8px;
  padding: 0.75rem 1rem; margin-bottom: 1.5rem;
}
.identical { color: var(--add-fg); font-weight: 600; }
.binary { color: var(--muted); }
code { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
details {
  border: 1px solid var(--border); border-radius: 8px; margin-bottom: 1rem; overflow: hidden;
}
summary {
  cursor: pointer; padding: 0.6rem 1rem; background: var(--card-bg); font-weight: 600;
}
ul.filelist { list-style: none; margin: 0; padding: 0.5rem 1rem 0.75rem; }
ul.filelist li { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 13px; padding: 2px 0; }
.filelist-item { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 13px; padding: 0.5rem 1rem; }
.muted { color: var(--muted); }
.diff-scroll { overflow-x: auto; border: 1px solid var(--border); border-radius: 8px; margin: 0.5rem 0; }
.sbs-diff {
  width: 100%; border-collapse: collapse; table-layout: fixed;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 13px;
}
.sbs-diff col { width: 50%; }
.sbs-diff td {
  padding: 1px 0.75rem; white-space: pre-wrap; word-break: break-word; vertical-align: top;
  border-right: 1px solid var(--border);
}
.sbs-diff td:last-child { border-right: none; }
.sbs-diff td.hunk-header {
  color: var(--muted); background: var(--hunk-bg); font-size: 12px; border-right: none;
}
.sbs-diff td.ctx { background: transparent; }
.sbs-diff td.add { background: var(--add-bg); color: var(--add-fg); }
.sbs-diff td.del { background: var(--del-bg); color: var(--del-fg); }
.sbs-diff td.empty { background: var(--hunk-bg); }
.footer { color: var(--muted); font-size: 12px; margin-top: 2rem; }
"#;

fn html_page(a: &Path, b: &Path, body: &str) -> String {
    let title = format!("vaqum diff: {} vs {}", display_name(a), display_name(b));
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>{STYLE}</style>
</head>
<body>
<h1>{title}</h1>
{body}
<p class="footer">Generated by <code>vaqum diff</code></p>
</body>
</html>
"#,
        title = escape_html(&title),
    )
}

fn html_file_diff(
    name_a: &str,
    bytes_a: &[u8],
    name_b: &str,
    bytes_b: &[u8],
    identical: bool,
) -> String {
    let summary = format!(
        "<div class=\"summary\"><code>{}</code> vs <code>{}</code></div>",
        escape_html(name_a),
        escape_html(name_b)
    );
    let content = if identical {
        "<p class=\"identical\">✔ Files are identical.</p>".to_string()
    } else {
        match (as_text(bytes_a), as_text(bytes_b)) {
            (Some(ta), Some(tb)) => html_hunks(ta, tb),
            _ => format!(
                "<p class=\"binary\">Binary files differ — {} vs {}.</p>",
                human_bytes(bytes_a.len() as u64),
                human_bytes(bytes_b.len() as u64)
            ),
        }
    };
    format!("{summary}\n{content}")
}

fn html_tree_diff(name_a: &str, name_b: &str, diff: &TreeDiff) -> String {
    let mut out = format!(
        "<div class=\"summary\"><code>{}</code> vs <code>{}</code> — {} unchanged, {} added, {} removed, {} modified</div>",
        escape_html(name_a),
        escape_html(name_b),
        diff.unchanged_count,
        diff.added.len(),
        diff.removed.len(),
        diff.modified.len()
    );

    if diff.is_identical() {
        out.push_str("<p class=\"identical\">✔ Directory contents are identical.</p>");
        return out;
    }

    if !diff.removed.is_empty() {
        out.push_str(&format!(
            "<details open><summary>Removed ({})</summary><ul class=\"filelist\">",
            diff.removed.len()
        ));
        for f in &diff.removed {
            out.push_str(&format!(
                "<li>- {} <span class=\"muted\">({})</span></li>",
                escape_html(&f.rel_path),
                human_bytes(f.size)
            ));
        }
        out.push_str("</ul></details>");
    }

    if !diff.added.is_empty() {
        out.push_str(&format!(
            "<details open><summary>Added ({})</summary><ul class=\"filelist\">",
            diff.added.len()
        ));
        for f in &diff.added {
            out.push_str(&format!(
                "<li>+ {} <span class=\"muted\">({})</span></li>",
                escape_html(&f.rel_path),
                human_bytes(f.size)
            ));
        }
        out.push_str("</ul></details>");
    }

    if !diff.modified.is_empty() {
        out.push_str(&format!(
            "<details open><summary>Modified ({})</summary>",
            diff.modified.len()
        ));
        for f in &diff.modified {
            match &f.content {
                ModifiedContent::Text { text_a, text_b } => {
                    out.push_str(&format!(
                        "<details><summary>{}</summary>",
                        escape_html(&f.rel_path)
                    ));
                    out.push_str(&html_hunks(text_a, text_b));
                    out.push_str("</details>");
                }
                ModifiedContent::Binary { size_a, size_b } => {
                    out.push_str(&format!(
                        "<div class=\"filelist-item\">~ {} <span class=\"muted\">(binary, {} \u{2192} {})</span></div>",
                        escape_html(&f.rel_path),
                        human_bytes(*size_a),
                        human_bytes(*size_b)
                    ));
                }
            }
        }
        out.push_str("</details>");
    }

    out
}

/// A row in the rendered side-by-side table: either a line unchanged on
/// both sides, or an old/new pair where either half may be absent (a pure
/// addition or pure removal).
enum SbsRow {
    Equal(String),
    Change {
        old: Option<String>,
        new: Option<String>,
    },
}

/// Regroups a hunk's flat change stream (Equal/Delete/Insert, in order)
/// into side-by-side rows: consecutive delete-runs and insert-runs are
/// paired off line-by-line (padding the shorter run with blanks), which is
/// the same pairing every split-view diff (GitHub, VS Code, ...) uses.
fn pair_for_side_by_side(changes: &[(ChangeTag, String)]) -> Vec<SbsRow> {
    let mut rows = Vec::new();
    let mut dels: Vec<String> = Vec::new();
    let mut inss: Vec<String> = Vec::new();

    for (tag, text) in changes {
        match tag {
            ChangeTag::Delete => dels.push(text.clone()),
            ChangeTag::Insert => inss.push(text.clone()),
            ChangeTag::Equal => {
                flush_pending(&mut dels, &mut inss, &mut rows);
                rows.push(SbsRow::Equal(text.clone()));
            }
        }
    }
    flush_pending(&mut dels, &mut inss, &mut rows);
    rows
}

fn flush_pending(dels: &mut Vec<String>, inss: &mut Vec<String>, rows: &mut Vec<SbsRow>) {
    let paired = dels.len().max(inss.len());
    for i in 0..paired {
        rows.push(SbsRow::Change {
            old: dels.get(i).cloned(),
            new: inss.get(i).cloned(),
        });
    }
    dels.clear();
    inss.clear();
}

fn html_hunks(text_a: &str, text_b: &str) -> String {
    let diff = TextDiff::from_lines(text_a, text_b);
    let unified = diff.unified_diff();
    let mut out = String::from("<table class=\"sbs-diff\"><colgroup><col/><col/></colgroup>");
    let mut any = false;

    for hunk in unified.iter_hunks() {
        any = true;
        out.push_str(&format!(
            "<tr><td class=\"hunk-header\" colspan=\"2\">{}</td></tr>",
            escape_html(&hunk.header().to_string())
        ));

        let changes: Vec<(ChangeTag, String)> = hunk
            .iter_changes()
            .map(|c| {
                (
                    c.tag(),
                    c.to_string_lossy().trim_end_matches('\n').to_string(),
                )
            })
            .collect();

        for row in pair_for_side_by_side(&changes) {
            match row {
                SbsRow::Equal(text) => {
                    let cell = escape_html(&text);
                    out.push_str(&format!(
                        "<tr><td class=\"ctx\">{cell}</td><td class=\"ctx\">{cell}</td></tr>"
                    ));
                }
                SbsRow::Change { old, new } => {
                    let old_cell = old
                        .map(|t| format!("<td class=\"del\">{}</td>", escape_html(&t)))
                        .unwrap_or_else(|| "<td class=\"empty\"></td>".to_string());
                    let new_cell = new
                        .map(|t| format!("<td class=\"add\">{}</td>", escape_html(&t)))
                        .unwrap_or_else(|| "<td class=\"empty\"></td>".to_string());
                    out.push_str(&format!("<tr>{old_cell}{new_cell}</tr>"));
                }
            }
        }
    }

    out.push_str("</table>");
    if !any {
        return "<p class=\"muted\">No line-level differences (whitespace/newline only?).</p>"
            .to_string();
    }
    format!("<div class=\"diff-scroll\">{out}</div>")
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn open_in_browser(path: &Path) {
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(path).status()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .status()
    } else {
        std::process::Command::new("xdg-open").arg(path).status()
    };
    if let Err(err) = result {
        eprintln!("warning: could not open browser automatically: {err}");
    }
}
