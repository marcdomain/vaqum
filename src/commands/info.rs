use anyhow::Result;

use crate::cli::InfoArgs;
use crate::format::{EntryType, read_header_and_total_size};
use crate::util::{hex_encode, human_bytes};

pub fn run(args: InfoArgs) -> Result<()> {
    let (header, total_size) = read_header_and_total_size(&args.path)?;
    let compressed_size = total_size.saturating_sub(header.on_disk_len());
    let ratio = if compressed_size > 0 {
        header.original_size as f64 / compressed_size as f64
    } else {
        1.0
    };

    println!("{}", args.path.display());
    println!(
        "  type:         {}",
        match header.entry_type {
            EntryType::File => "single file",
            EntryType::Archive => "directory archive",
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

    Ok(())
}
