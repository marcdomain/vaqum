//! `--exclude`/`.vaqumignore` filtering for directory compression.
//!
//! A pattern ending in `/` excludes a directory (and everything under
//! it); without the trailing `/` it excludes a file. A pattern with no
//! other `/` matches by that name at any depth; one with an internal `/`
//! matches the path relative to the directory root. No negation.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};

pub const IGNORE_FILE: &str = ".vaqumignore";

pub struct ExcludeSet {
    dirs: Option<GlobSet>,
    files: Option<GlobSet>,
}

impl ExcludeSet {
    /// Build from `--exclude` patterns plus any `.vaqumignore` found at
    /// `root`. An all-empty result matches nothing.
    pub fn build(root: &Path, cli_patterns: &[String]) -> Result<Self> {
        let mut patterns: Vec<String> = cli_patterns.to_vec();
        let ignore_file = root.join(IGNORE_FILE);
        if ignore_file.is_file() {
            let contents = fs::read_to_string(&ignore_file)
                .with_context(|| format!("failed to read {}", ignore_file.display()))?;
            for line in contents.lines() {
                let line = line.trim();
                if !line.is_empty() && !line.starts_with('#') {
                    patterns.push(line.to_string());
                }
            }
        }

        let mut dir_builder = GlobSetBuilder::new();
        let mut file_builder = GlobSetBuilder::new();
        let mut has_dirs = false;
        let mut has_files = false;
        for pattern in &patterns {
            if let Some(stripped) = pattern.strip_suffix('/') {
                add_pattern(&mut dir_builder, stripped)?;
                has_dirs = true;
            } else {
                add_pattern(&mut file_builder, pattern)?;
                has_files = true;
            }
        }

        Ok(Self {
            dirs: has_dirs.then(|| dir_builder.build()).transpose()?,
            files: has_files.then(|| file_builder.build()).transpose()?,
        })
    }

    pub fn is_excluded(&self, rel_posix_path: &str, is_dir: bool) -> bool {
        let set = if is_dir { &self.dirs } else { &self.files };
        set.as_ref().is_some_and(|s| s.is_match(rel_posix_path))
    }
}

fn add_pattern(builder: &mut GlobSetBuilder, pattern: &str) -> Result<()> {
    let compile = |p: &str| Glob::new(p).with_context(|| format!("invalid exclude pattern '{p}'"));
    builder.add(compile(pattern)?);
    if pattern.contains('/') {
        builder.add(compile(&format!("{pattern}/**"))?);
    } else {
        builder.add(compile(&format!("**/{pattern}"))?);
    }
    Ok(())
}
