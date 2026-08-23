//! `vaqum shred` — best-effort secure deletion.
//!
//! Multi-pass overwrite is irreversible by design and, on SSDs, not
//! forensic-grade (wear-leveling/TRIM mean the drive may retain copies of
//! data elsewhere on the flash regardless of what gets overwritten at the
//! logical block address). We say so up front rather than overpromising.

use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rand::Rng;

use crate::cli::ShredArgs;
use crate::util::{human_bytes, prompt_line};

const CHUNK_SIZE: usize = 1024 * 1024;

/// One overwrite target: an absolute path and its current length.
type Target = (PathBuf, u64);

pub fn run(args: ShredArgs) -> Result<()> {
    if !args.path.exists() {
        bail!("'{}' does not exist", args.path.display());
    }
    let is_dir = args.path.is_dir();
    if is_dir && !args.recursive {
        bail!(
            "'{}' is a directory; pass -r/--recursive to shred it",
            args.path.display()
        );
    }

    let targets = collect_targets(&args.path, is_dir)?;
    let total_size: u64 = targets.iter().map(|(_, len)| *len).sum();

    print_warning(&args.path, &targets, total_size, is_dir);

    if args.dry_run {
        println!("(dry run) nothing was shredded.");
        return Ok(());
    }

    if !args.yes {
        println!();
        let confirm_token = args
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| args.path.display().to_string());
        let answer = prompt_line("Type the filename to confirm: ")?;
        if answer != confirm_token && !answer.eq_ignore_ascii_case("yes") {
            bail!("Aborted: confirmation did not match. Nothing was shredded.");
        }
    }

    for (path, _len) in &targets {
        shred_file(path, args.passes)
            .with_context(|| format!("failed to shred {}", path.display()))?;
    }

    if is_dir {
        fs::remove_dir_all(&args.path)
            .with_context(|| format!("failed to remove directory {}", args.path.display()))?;
    }

    println!(
        "✔ Shredded ({} pass{}, DoD 5220.22-M-style overwrite)",
        args.passes,
        if args.passes == 1 { "" } else { "es" }
    );
    println!(
        "  note: on SSDs, overwrite passes are best-effort (wear-leveling/TRIM) — not forensic-grade."
    );

    Ok(())
}

fn collect_targets(root: &Path, is_dir: bool) -> Result<Vec<Target>> {
    if !is_dir {
        let len = fs::metadata(root)
            .with_context(|| format!("failed to stat {}", root.display()))?
            .len();
        return Ok(vec![(root.to_path_buf(), len)]);
    }

    let mut targets = Vec::new();
    for entry in walkdir::WalkDir::new(root).min_depth(1) {
        let entry = entry.with_context(|| format!("failed to walk {}", root.display()))?;
        if entry.file_type().is_file() {
            let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
            targets.push((entry.path().to_path_buf(), len));
        }
    }
    Ok(targets)
}

fn print_warning(root: &Path, targets: &[Target], total_size: u64, is_dir: bool) {
    println!("⚠  This will permanently and irrecoverably destroy:");
    if is_dir {
        println!(
            "    {} ({} file{}, {})",
            root.display(),
            targets.len(),
            if targets.len() == 1 { "" } else { "s" },
            human_bytes(total_size)
        );
    } else {
        println!("    {} ({})", root.display(), human_bytes(total_size));
    }
}

/// Overwrite a file's content `passes` times, then truncate and unlink it.
fn shred_file(path: &Path, passes: u32) -> Result<()> {
    let len = fs::metadata(path)
        .with_context(|| format!("failed to stat {}", path.display()))?
        .len();
    let passes = passes.max(1);

    let mut file = OpenOptions::new()
        .write(true)
        .open(path)
        .with_context(|| format!("failed to open {} for overwrite", path.display()))?;

    for pass in 0..passes {
        // Approximates DoD 5220.22-M's three-pass scheme (zeros, ones,
        // random) and extends sensibly to other pass counts: the final
        // pass is always random, earlier ones alternate 0x00/0xFF.
        let pattern = if pass == passes - 1 {
            Pattern::Random
        } else if pass % 2 == 0 {
            Pattern::Zeros
        } else {
            Pattern::Ones
        };
        overwrite_pass(&mut file, len, pattern)
            .with_context(|| format!("overwrite pass {} on {}", pass + 1, path.display()))?;
    }

    file.set_len(0)
        .with_context(|| format!("failed to truncate {}", path.display()))?;
    let _ = file.sync_all();
    drop(file);

    fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))?;
    Ok(())
}

enum Pattern {
    Zeros,
    Ones,
    Random,
}

fn overwrite_pass(file: &mut File, len: u64, pattern: Pattern) -> Result<()> {
    file.seek(SeekFrom::Start(0))?;

    let buf_len = CHUNK_SIZE.min(len.max(1) as usize);
    let mut buf = vec![0u8; buf_len];
    match pattern {
        Pattern::Zeros => buf.fill(0x00),
        Pattern::Ones => buf.fill(0xFF),
        Pattern::Random => {}
    }

    let mut rng = rand::rng();
    let mut remaining = len;
    while remaining > 0 {
        let n = (remaining as usize).min(buf.len());
        if matches!(pattern, Pattern::Random) {
            rng.fill_bytes(&mut buf[..n]);
        }
        file.write_all(&buf[..n])?;
        remaining -= n as u64;
    }

    file.flush()?;
    file.sync_all()?;
    Ok(())
}
