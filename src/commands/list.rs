//! `vaqum list` — show what's inside an archive without extracting it.
//! Works on `.vaqum` (including encrypted and dedup archives), and on the
//! zip / tar.gz archives `decompress` can read.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use serde::Serialize;

use crate::archive::{self, Kind, ListEntry};
use crate::cli::ListArgs;
use crate::foreign::{self, ForeignFormat};
use crate::format::{self, EntryType};
use crate::progress::{self, ProgressReader};
use crate::risk::{self, Warning};
use crate::util::{human_bytes, is_stdio, to_posix_path};

#[derive(Serialize)]
struct Listing<'a> {
    archive: String,
    format: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<&'a str>,
    encrypted: bool,
    files: u64,
    dirs: u64,
    total_size: u64,
    entries: Vec<ListEntry>,
}

pub fn run(args: ListArgs) -> Result<()> {
    let path = &args.path;
    let is_vaqum = is_stdio(path) || format::is_vaqum_file(path)?;

    let (format_label, kind, encrypted, entries) = if is_vaqum {
        let src = archive::open_vaqum(path)?;
        let header = src.header;
        let bar = progress::bar(header.original_size, args.quiet || args.json, "Reading");
        let payload = archive::decoded_payload(
            src.reader,
            &header,
            archive::KeyOptions {
                key_file: args.key_file.as_deref(),
                identity: args.identity.as_deref(),
            },
        )?;
        let entries = archive::list_vaqum(ProgressReader::new(payload, bar.clone()), &header)?;
        bar.finish_and_clear();
        let kind = match header.entry_type {
            EntryType::File => "single file",
            EntryType::Archive => "directory archive",
            EntryType::Bundle => "multi-path bundle",
        };
        ("vaqum", Some(kind), header.encrypted, entries)
    } else if let Some(fmt) = foreign::detect(path)? {
        (fmt.label(), None, false, list_foreign(path, fmt)?)
    } else {
        bail!("'{}' is not a .vqm, zip, or tar.gz archive", path.display());
    };

    let files = entries.iter().filter(|e| e.kind == Kind::File).count() as u64;
    let dirs = entries.iter().filter(|e| e.kind == Kind::Dir).count() as u64;
    let total_size: u64 = entries.iter().map(|e| e.size).sum();

    if args.json {
        let listing = Listing {
            archive: path.display().to_string(),
            format: format_label,
            kind,
            encrypted,
            files,
            dirs,
            total_size,
            entries,
        };
        println!("{}", serde_json::to_string_pretty(&listing)?);
        return Ok(());
    }

    for e in &entries {
        let flag = if e.warnings.is_empty() { " " } else { "⚠" };
        match e.kind {
            Kind::File => println!("{:>12} {flag} {}", human_bytes(e.size), e.path),
            Kind::Dir => println!("{:>12} {flag} {}/", "-", e.path),
            Kind::Link => println!(
                "{:>12} {flag} {} -> {}",
                "link",
                e.path,
                e.target.as_deref().unwrap_or("")
            ),
        }
    }
    println!(
        "{files} file{}, {dirs} director{}, {} total",
        if files == 1 { "" } else { "s" },
        if dirs == 1 { "y" } else { "ies" },
        human_bytes(total_size)
    );
    let warnings: Vec<Warning> = entries
        .iter()
        .flat_map(|e| {
            e.warnings.iter().map(|r| Warning {
                path: e.path.clone(),
                reason: r.clone(),
            })
        })
        .collect();
    risk::print(&warnings);
    Ok(())
}

fn list_foreign(path: &Path, fmt: ForeignFormat) -> Result<Vec<ListEntry>> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut out = Vec::new();
    match fmt {
        ForeignFormat::Zip => {
            let mut zip = zip::ZipArchive::new(file)
                .with_context(|| format!("failed to read zip archive {}", path.display()))?;
            for i in 0..zip.len() {
                let mut entry = zip.by_index(i)?;
                let name =
                    archive::normalize(&entry.name().context("zip entry has an unreadable name")?);
                if entry.is_dir() {
                    out.push(ListEntry::dir(name));
                } else if entry.is_symlink() {
                    let mut target = String::new();
                    entry.read_to_string(&mut target).ok();
                    out.push(ListEntry::link(name, target, false));
                } else {
                    let size = entry.size();
                    let mode = entry.unix_mode();
                    let head = archive::read_head(&mut entry);
                    out.push(ListEntry::file(name, size, mode, &head));
                }
            }
        }
        ForeignFormat::TarGz => {
            let mut tar = tar::Archive::new(GzDecoder::new(file));
            for entry in tar.entries()? {
                let mut entry = entry?;
                let rel = archive::normalize(&to_posix_path(&entry.path()?));
                if rel.is_empty() {
                    continue;
                }
                if let Some(row) = archive::tar_list_entry(&mut entry, rel) {
                    out.push(row);
                }
            }
        }
    }
    Ok(out)
}
