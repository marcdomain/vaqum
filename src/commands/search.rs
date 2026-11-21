//! `vaqum search` — find files by name and/or by content, recursively.
//!
//! Follows the classic `grep` exit-code contract (0 = matches found, 1 =
//! none, 2 = trouble), same convention `diff` already uses. Every result
//! line is tagged with its kind so a `name` hit (the path itself matched)
//! is never confused with a `content` hit (a line inside the file
//! matched) — see `main.rs` for the exit-code wiring and `SearchArgs`'s
//! `after_help` for the exact output format.

use std::fs;
use std::io::{self, BufRead, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use regex::{Regex, RegexBuilder};

use crate::cli::SearchArgs;

pub fn run(args: SearchArgs) -> Result<bool> {
    let root = args.path.clone().unwrap_or_else(|| PathBuf::from("."));
    if !root.exists() {
        bail!("'{}' does not exist", root.display());
    }

    let matcher = Matcher::new(&args.pattern, args.regex, args.ignore_case)?;
    let mode = match (args.names_only, args.content_only) {
        (true, _) => Mode::NamesOnly,
        (_, true) => Mode::ContentOnly,
        (false, false) => Mode::Both,
    };
    let threads = args.threads.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    });

    let all_paths = collect_paths(&root);

    let mut name_hits: Vec<PathBuf> = Vec::new();
    if !matches!(mode, Mode::ContentOnly) {
        for path in &all_paths {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if matcher.is_match(&name) {
                name_hits.push(path.clone());
            }
        }
        name_hits.sort();
    }

    let mut content_hits: Vec<(PathBuf, Vec<(usize, String)>)> = Vec::new();
    if !matches!(mode, Mode::NamesOnly) {
        let candidates: Vec<&PathBuf> = all_paths.iter().filter(|p| p.is_file()).collect();
        let pool = crate::util::build_thread_pool(Some(threads))?;
        let bar = crate::progress::counter(false, "Searching");
        content_hits = pool.install(|| {
            use rayon::prelude::*;
            candidates
                .par_iter()
                .inspect(|_| bar.inc(1))
                .filter_map(|path| match search_file_content(path, &matcher) {
                    Ok(lines) if !lines.is_empty() => Some(((*path).clone(), lines)),
                    Ok(_) => None,
                    Err(err) => {
                        eprintln!("warning: skipping {}: {err:#}", path.display());
                        None
                    }
                })
                .collect()
        });
        bar.finish_and_clear();
        content_hits.sort_by(|a, b| a.0.cmp(&b.0));
    }

    for path in &name_hits {
        println!("name     {}", path.display());
    }
    for (path, lines) in &content_hits {
        for (line_no, text) in lines {
            println!("content  {}:{line_no}: {}", path.display(), text.trim());
        }
    }

    let content_line_count: usize = content_hits.iter().map(|(_, lines)| lines.len()).sum();
    let total = name_hits.len() + content_line_count;
    if total == 0 {
        println!("(no matches under {})", root.display());
    } else {
        println!(
            "\n{total} match{} ({} file{} by name, {} line{} of content)",
            if total == 1 { "" } else { "es" },
            name_hits.len(),
            if name_hits.len() == 1 { "" } else { "s" },
            content_line_count,
            if content_line_count == 1 { "" } else { "s" },
        );
    }

    Ok(total > 0)
}

/// Walk `root` (or just return it, if it's a single file), tolerating
/// per-entry errors (e.g. a permission-denied subdirectory) by warning
/// and skipping rather than aborting the whole scan.
fn collect_paths(root: &Path) -> Vec<PathBuf> {
    if root.is_file() {
        return vec![root.to_path_buf()];
    }
    let mut paths = Vec::new();
    for entry in walkdir::WalkDir::new(root) {
        match entry {
            Ok(entry) => paths.push(entry.into_path()),
            Err(err) => eprintln!("warning: {err}"),
        }
    }
    paths
}

/// Lines longer than this mark a file as not-really-text (e.g. a huge
/// minified blob), so searching it can't exhaust memory.
const MAX_LINE: u64 = 16 * 1024 * 1024;

/// Streams `path` line by line, in constant memory however large it is.
/// Same rules as before for what counts as text: no NUL byte in the first
/// 8000 bytes, valid UTF-8 throughout. Non-text files yield no matches.
fn search_file_content(path: &Path, matcher: &Matcher) -> Result<Vec<(usize, String)>> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = io::BufReader::with_capacity(256 * 1024, file);
    if reader.fill_buf()?.iter().take(8000).any(|&b| b == 0) {
        return Ok(Vec::new());
    }
    let mut hits = Vec::new();
    let mut line = Vec::new();
    let mut number = 0usize;
    loop {
        line.clear();
        let read = (&mut reader)
            .take(MAX_LINE + 1)
            .read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        if read as u64 > MAX_LINE {
            return Ok(Vec::new());
        }
        number += 1;
        let Ok(text) = std::str::from_utf8(&line) else {
            return Ok(Vec::new());
        };
        let text = text.strip_suffix('\n').unwrap_or(text);
        let text = text.strip_suffix('\r').unwrap_or(text);
        if matcher.is_match(text) {
            hits.push((number, text.to_string()));
        }
    }
    Ok(hits)
}

enum Mode {
    NamesOnly,
    ContentOnly,
    Both,
}

enum Matcher {
    Literal { needle: String, ignore_case: bool },
    Regex(Regex),
}

impl Matcher {
    fn new(pattern: &str, regex: bool, ignore_case: bool) -> Result<Self> {
        if regex {
            let re = RegexBuilder::new(pattern)
                .case_insensitive(ignore_case)
                .build()
                .with_context(|| format!("invalid regex: {pattern}"))?;
            Ok(Matcher::Regex(re))
        } else {
            Ok(Matcher::Literal {
                needle: pattern.to_string(),
                ignore_case,
            })
        }
    }

    fn is_match(&self, haystack: &str) -> bool {
        match self {
            Matcher::Literal {
                needle,
                ignore_case,
            } => {
                if *ignore_case {
                    haystack.to_lowercase().contains(&needle.to_lowercase())
                } else {
                    haystack.contains(needle.as_str())
                }
            }
            Matcher::Regex(re) => re.is_match(haystack),
        }
    }
}
