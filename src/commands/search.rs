//! `vaqum search` — find files by name and/or by content, recursively.
//!
//! Follows the classic `grep` exit-code contract (0 = matches found, 1 =
//! none, 2 = trouble), same convention `diff` already uses. Every result
//! line is tagged with its kind so a `name` hit (the path itself matched)
//! is never confused with a `content` hit (a line inside the file
//! matched) — see `main.rs` for the exit-code wiring and `SearchArgs`'s
//! `after_help` for the exact output format.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use regex::{Regex, RegexBuilder};

use crate::cli::SearchArgs;
use crate::util::as_text;

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
        content_hits = pool.install(|| {
            use rayon::prelude::*;
            candidates
                .par_iter()
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

fn search_file_content(path: &Path, matcher: &Matcher) -> Result<Vec<(usize, String)>> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let Some(text) = as_text(&bytes) else {
        return Ok(Vec::new());
    };
    Ok(text
        .lines()
        .enumerate()
        .filter(|(_, line)| matcher.is_match(line))
        .map(|(i, line)| (i + 1, line.to_string()))
        .collect())
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
