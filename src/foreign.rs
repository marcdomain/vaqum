//! Read-only support for `.zip` and `.tar.gz`/`.tgz` — foreign formats
//! `decompress`/`info` can open as a convenience, detected by content the
//! same way `.vaqum` itself is. vaqum never writes these formats; its own
//! format stays the only supported output.

use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use flate2::read::GzDecoder;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ForeignFormat {
    Zip,
    TarGz,
}

impl ForeignFormat {
    pub fn label(self) -> &'static str {
        match self {
            ForeignFormat::Zip => "zip",
            ForeignFormat::TarGz => "tar.gz",
        }
    }
}

const ZIP_MAGICS: [[u8; 4]; 3] = [*b"PK\x03\x04", *b"PK\x05\x06", *b"PK\x07\x08"];
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

pub fn detect(path: &Path) -> Result<Option<ForeignFormat>> {
    let mut f = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut magic = [0u8; 4];
    let mut n = 0;
    while n < magic.len() {
        match f.read(&mut magic[n..])? {
            0 => break,
            read => n += read,
        }
    }
    if n >= 4 && ZIP_MAGICS.contains(&magic) {
        return Ok(Some(ForeignFormat::Zip));
    }
    if n >= 2 && magic[..2] == GZIP_MAGIC {
        return Ok(Some(ForeignFormat::TarGz));
    }
    Ok(None)
}

pub struct ForeignStats {
    pub file_count: u64,
    pub uncompressed_size: u64,
    pub compressed_size: u64,
}

pub fn inspect(path: &Path, format: ForeignFormat) -> Result<ForeignStats> {
    let compressed_size = fs::metadata(path)
        .with_context(|| format!("failed to stat {}", path.display()))?
        .len();
    let (file_count, uncompressed_size) = match format {
        ForeignFormat::Zip => inspect_zip(path)?,
        ForeignFormat::TarGz => inspect_tar_gz(path)?,
    };
    Ok(ForeignStats {
        file_count,
        uncompressed_size,
        compressed_size,
    })
}

fn inspect_zip(path: &Path) -> Result<(u64, u64)> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("failed to read zip archive {}", path.display()))?;
    let mut file_count = 0u64;
    let mut uncompressed_size = 0u64;
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .with_context(|| format!("failed to read entry {i} of {}", path.display()))?;
        if !entry.is_dir() {
            file_count += 1;
            uncompressed_size += entry.size();
        }
    }
    Ok((file_count, uncompressed_size))
}

fn inspect_tar_gz(path: &Path) -> Result<(u64, u64)> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    let mut file_count = 0u64;
    let mut uncompressed_size = 0u64;
    let entries = archive
        .entries()
        .with_context(|| format!("failed to read tar.gz archive {}", path.display()))?;
    for entry in entries {
        let entry =
            entry.with_context(|| format!("failed to read an entry of {}", path.display()))?;
        if entry.header().entry_type().is_file() {
            file_count += 1;
            uncompressed_size += entry.header().size().unwrap_or(0);
        }
    }
    Ok((file_count, uncompressed_size))
}

pub fn extract(path: &Path, format: ForeignFormat, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest).with_context(|| format!("failed to create {}", dest.display()))?;
    match format {
        ForeignFormat::Zip => extract_zip(path, dest),
        ForeignFormat::TarGz => extract_tar_gz(path, dest),
    }
}

fn extract_zip(path: &Path, dest: &Path) -> Result<()> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("failed to read zip archive {}", path.display()))?;
    archive
        .extract(dest)
        .with_context(|| format!("failed to extract zip archive into {}", dest.display()))?;
    Ok(())
}

fn extract_tar_gz(path: &Path, dest: &Path) -> Result<()> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    tar::Archive::new(GzDecoder::new(file))
        .unpack(dest)
        .with_context(|| format!("failed to extract tar.gz archive into {}", dest.display()))?;
    Ok(())
}
