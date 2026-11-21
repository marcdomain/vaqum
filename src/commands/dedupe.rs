//! `vaqum dedupe` — standalone duplicate-file finder for a directory tree
//! (e.g. a photo library), independent of `compress --dedup`. Reports by
//! default; `--link` actually reclaims the space via hardlinks, which is
//! safe and reversible (breaking or replacing a hardlink never touches the
//! other copies) unlike deleting duplicates outright.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::cli::DedupeArgs;
use crate::progress;
use crate::util::{TreeFile, hash_tree, hex_encode, human_bytes};

pub fn run(args: DedupeArgs) -> Result<()> {
    if !args.path.is_dir() {
        bail!("'{}' is not a directory", args.path.display());
    }
    let threads = args.threads.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    });

    let total_bytes: u64 = walkdir::WalkDir::new(&args.path)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
        .sum();
    let bar = progress::bar(total_bytes, args.quiet, "Scanning");
    let (_, files) = hash_tree(&args.path, threads, None, Some(&bar))?;
    bar.finish_and_clear();

    let mut groups: HashMap<[u8; 32], Vec<&TreeFile>> = HashMap::new();
    for file in &files {
        groups.entry(file.hash).or_default().push(file);
    }
    let mut duplicate_groups: Vec<&Vec<&TreeFile>> =
        groups.values().filter(|g| g.len() > 1).collect();
    duplicate_groups.sort_by(|a, b| a[0].rel_path.cmp(&b[0].rel_path));

    let total_size: u64 = files.iter().map(|f| f.size).sum();
    let duplicate_file_count: usize = duplicate_groups.iter().map(|g| g.len() - 1).sum();
    let reclaimable: u64 = duplicate_groups
        .iter()
        .map(|g| g[0].size * (g.len() as u64 - 1))
        .sum();

    println!("{}", args.path.display());
    println!(
        "  {:<18} {} ({})",
        "files scanned:",
        files.len(),
        human_bytes(total_size)
    );
    println!(
        "  {:<18} {} ({duplicate_file_count} redundant file{})",
        "duplicate groups:",
        duplicate_groups.len(),
        if duplicate_file_count == 1 { "" } else { "s" }
    );
    println!("  {:<18} {}", "reclaimable:", human_bytes(reclaimable));

    if args.verbose {
        for group in &duplicate_groups {
            println!(
                "\n  sha256:{} ({}, {} copies)",
                hex_encode(&group[0].hash),
                human_bytes(group[0].size),
                group.len()
            );
            for file in group.iter() {
                println!("    {}", file.rel_path);
            }
        }
    }

    if args.link {
        link_duplicates(
            &duplicate_groups,
            duplicate_file_count,
            reclaimable,
            args.dry_run,
        )?;
    }

    Ok(())
}

fn link_duplicates(
    duplicate_groups: &[&Vec<&TreeFile>],
    duplicate_file_count: usize,
    reclaimable: u64,
    dry_run: bool,
) -> Result<()> {
    if dry_run {
        println!(
            "\n(dry run) would hardlink {duplicate_file_count} duplicate file(s), reclaiming {}",
            human_bytes(reclaimable)
        );
        return Ok(());
    }

    let mut linked = 0u64;
    let mut linked_bytes = 0u64;
    let mut skipped = 0u64;
    for group in duplicate_groups {
        let (first, rest) = group
            .split_first()
            .expect("duplicate groups always have at least 2 entries");
        for dup in rest {
            if same_file(&first.abs_path, &dup.abs_path) {
                continue; // already one file under two names
            }
            match relink(&first.abs_path, &dup.abs_path) {
                Ok(true) => {
                    linked += 1;
                    linked_bytes += dup.size;
                }
                Ok(false) => {
                    skipped += 1;
                    eprintln!(
                        "warning: skipped {}: it changed since the scan, so it's no longer a duplicate",
                        dup.rel_path
                    );
                }
                Err(err) => {
                    skipped += 1;
                    eprintln!("warning: failed to hardlink {}: {err:#}", dup.rel_path);
                }
            }
        }
    }

    println!(
        "\n✔ Hardlinked {linked} duplicate file(s), reclaiming {}",
        human_bytes(linked_bytes)
    );
    if skipped > 0 {
        println!(
            "  ({skipped} could not be linked — see warnings above; likely a different filesystem)"
        );
    }
    Ok(())
}

/// Replace `dup` with a hardlink to `original`, via a temp name + rename so
/// a failure partway through never leaves `dup` missing. The two files are
/// compared byte for byte first: if either changed since the scan, nothing
/// is replaced and this returns `Ok(false)`, so an edit made in between is
/// never lost.
fn relink(original: &Path, dup: &Path) -> Result<bool> {
    if !identical(original, dup)? {
        return Ok(false);
    }
    let mut tmp_name = dup.as_os_str().to_os_string();
    tmp_name.push(".vaqum-link-tmp");
    let tmp = PathBuf::from(tmp_name);

    fs::hard_link(original, &tmp).context("failed to create hardlink")?;
    if let Err(err) = fs::rename(&tmp, dup) {
        let _ = fs::remove_file(&tmp);
        return Err(err).context("failed to swap in the hardlink");
    }
    Ok(true)
}

/// Streams both files and compares them byte for byte.
fn identical(a: &Path, b: &Path) -> Result<bool> {
    use std::io::Read;
    let (fa, fb) = (fs::File::open(a)?, fs::File::open(b)?);
    if fa.metadata()?.len() != fb.metadata()?.len() {
        return Ok(false);
    }
    let mut ra = std::io::BufReader::with_capacity(256 * 1024, fa);
    let mut rb = std::io::BufReader::with_capacity(256 * 1024, fb);
    let (mut ba, mut bb) = (vec![0u8; 256 * 1024], vec![0u8; 256 * 1024]);
    loop {
        let n = ra.read(&mut ba)?;
        if n == 0 {
            return Ok(rb.read(&mut bb[..1])? == 0);
        }
        rb.read_exact(&mut bb[..n])?;
        if ba[..n] != bb[..n] {
            return Ok(false);
        }
    }
}

/// Whether two paths are already the same file (hardlinked together).
fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (fs::metadata(a), fs::metadata(b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        match (a.canonicalize(), b.canonicalize()) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        }
    }
}
