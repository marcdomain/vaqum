use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::cli::InfoArgs;
use crate::foreign::{self, ForeignFormat};
use crate::format::{self, EntryType, read_header_and_total_size};
use crate::util::{format_time, hash_file, hex_encode, human_bytes};

pub fn run(args: InfoArgs) -> Result<()> {
    let path = &args.path;
    if !path.exists() {
        bail!("'{}' does not exist", path.display());
    }

    if path.is_dir() {
        show_dir_info(path)
    } else if format::is_vaqum_file(path)? {
        show_vaqum_info(path)
    } else if let Some(fmt) = foreign::detect(path)? {
        show_foreign_info(path, fmt)
    } else {
        show_file_info(path)
    }
}

fn show_vaqum_info(path: &Path) -> Result<()> {
    let (header, total_size) = read_header_and_total_size(path)?;
    let compressed_size = total_size.saturating_sub(header.on_disk_len());
    let ratio = if compressed_size > 0 {
        header.original_size as f64 / compressed_size as f64
    } else {
        1.0
    };

    println!("{}", path.display());
    println!(
        "  type:         {}",
        match header.entry_type {
            EntryType::File => "single file",
            EntryType::Archive => "directory archive",
            EntryType::Bundle => "multi-path bundle",
        }
    );
    println!("  name:         {}", header.name);
    println!(
        "  algorithm:    {}{}",
        header.algorithm.label(),
        if header.dedup { " + dedup" } else { "" }
    );
    println!("  original:     {}", human_bytes(header.original_size));
    println!(
        "  compressed:   {} ({:.2}x)",
        human_bytes(compressed_size),
        ratio
    );
    println!("  on disk:      {}", human_bytes(total_size));
    println!("  checksum:     sha256:{}", hex_encode(&header.checksum));
    println!(
        "  encrypted:    {}",
        if header.encrypted { "yes" } else { "no" }
    );

    Ok(())
}

fn show_foreign_info(path: &Path, fmt: ForeignFormat) -> Result<()> {
    let stats = foreign::inspect(path, fmt)?;
    let ratio = if stats.compressed_size > 0 {
        stats.uncompressed_size as f64 / stats.compressed_size as f64
    } else {
        1.0
    };

    println!("{}", path.display());
    println!("  type:         {} archive", fmt.label());
    println!("  files:        {}", stats.file_count);
    println!("  uncompressed: {}", human_bytes(stats.uncompressed_size));
    println!(
        "  compressed:   {} ({:.2}x)",
        human_bytes(stats.compressed_size),
        ratio
    );

    Ok(())
}

fn show_file_info(path: &Path) -> Result<()> {
    let (size, checksum) =
        hash_file(path).with_context(|| format!("failed to read {}", path.display()))?;

    println!("{}", path.display());
    println!("  type:         file");
    println!("  size:         {} ({size} bytes)", human_bytes(size));
    print_timestamps(path)?;
    println!("  checksum:     sha256:{}", hex_encode(&checksum));

    Ok(())
}

fn show_dir_info(path: &Path) -> Result<()> {
    let mut files = 0u64;
    let mut dirs = 0u64;
    let mut total_size = 0u64;
    for entry in walkdir::WalkDir::new(path).min_depth(1) {
        let entry = entry.with_context(|| format!("failed to walk {}", path.display()))?;
        if entry.file_type().is_dir() {
            dirs += 1;
        } else if entry.file_type().is_file() {
            files += 1;
            total_size += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }

    println!("{}", path.display());
    println!("  type:         directory");
    println!(
        "  size:         {} ({total_size} bytes)",
        human_bytes(total_size)
    );
    println!("  files:        {files}");
    println!("  directories:  {dirs}");
    print_timestamps(path)?;

    Ok(())
}

/// Prints `created`/`modified` (RFC 3339, UTC) for `path`'s own metadata.
/// `created` is silently omitted where the platform/filesystem doesn't
/// track birth time.
fn print_timestamps(path: &Path) -> Result<()> {
    let metadata = path
        .metadata()
        .with_context(|| format!("failed to stat {}", path.display()))?;
    if let Ok(created) = metadata.created() {
        println!("  created:      {}", format_time(created));
    }
    if let Ok(modified) = metadata.modified() {
        println!("  modified:     {}", format_time(modified));
    }
    Ok(())
}
