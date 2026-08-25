//! Archive bomb safety check, run before decompression starts.

use std::path::Path;

use anyhow::{Result, bail};

use crate::util::human_bytes;

pub const DEFAULT_MAX_RATIO: f64 = 1000.0;
const WARN_RATIO: f64 = 100.0;

/// Aborts (unless `force`) if the claimed expansion ratio exceeds
/// `max_ratio`, or if decompressing would exceed available disk space.
/// Warns, but doesn't block, past [`WARN_RATIO`].
pub fn check(
    original_size: u64,
    compressed_size: u64,
    dest_dir: &Path,
    max_ratio: f64,
    force: bool,
) -> Result<()> {
    let ratio = if compressed_size > 0 {
        original_size as f64 / compressed_size as f64
    } else {
        original_size as f64
    };

    if ratio > max_ratio {
        if force {
            eprintln!(
                "⚠  This archive claims to expand from {} to {} (ratio {})",
                human_bytes(compressed_size),
                human_bytes(original_size),
                format_ratio(ratio)
            );
            eprintln!("    --force set, proceeding anyway.");
        } else {
            bail!(
                "⚠  This archive claims to expand from {} to {} (ratio {})\n\
                 \x20   This is far beyond normal compression ratios and may be a decompression bomb.\n\
                 \x20   Refusing to proceed. Use --force to override.",
                human_bytes(compressed_size),
                human_bytes(original_size),
                format_ratio(ratio)
            );
        }
    } else if ratio > WARN_RATIO {
        eprintln!(
            "⚠  This archive expands from {} to {} (ratio {}). Proceeding.",
            human_bytes(compressed_size),
            human_bytes(original_size),
            format_ratio(ratio)
        );
    }

    if !force {
        check_disk_space(original_size, dest_dir)?;
    }

    Ok(())
}

fn check_disk_space(original_size: u64, dest_dir: &Path) -> Result<()> {
    let dir = if dest_dir.exists() {
        dest_dir.to_path_buf()
    } else {
        dest_dir
            .ancestors()
            .find(|p| p.exists())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
    };
    let available = fs2::available_space(&dir)?;
    if original_size > available {
        bail!(
            "decompressing would need {} but only {} is available on disk at '{}'. Use --force to override.",
            human_bytes(original_size),
            human_bytes(available),
            dir.display()
        );
    }
    Ok(())
}

/// e.g. 240000.0 -> "240,000:1"
fn format_ratio(ratio: f64) -> String {
    let digits = (ratio.round() as u64).to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{grouped}:1")
}
