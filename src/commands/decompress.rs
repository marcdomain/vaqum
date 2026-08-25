use std::env;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::bomb;
use crate::cli::DecompressArgs;
use crate::codec;
use crate::crypto;
use crate::dedup;
use crate::format::{EntryType, HashingWriter, Header};
use crate::util::{hex_encode, human_bytes};

pub fn run(args: DecompressArgs) -> Result<()> {
    let mut in_file = File::open(&args.path)
        .with_context(|| format!("failed to open {}", args.path.display()))?;
    let total_size = in_file.metadata()?.len();
    let header = Header::read(&mut in_file)?;
    let compressed_size = total_size.saturating_sub(header.on_disk_len());

    let dest_dir = bomb_check_dir(&header, &args)?;
    bomb::check(
        header.original_size,
        compressed_size,
        &dest_dir,
        args.max_ratio,
        args.force,
    )?;

    let key = header
        .encrypted
        .then(|| resolve_decrypt_key(&args, &header))
        .transpose()?;
    let key_ref = key.as_ref().map(|k| (k, &header.nonce_prefix));
    let decrypt_reader = crypto::DecryptReader::new(in_file, key_ref);
    let decoder = codec::Decoder::new(decrypt_reader, header.algorithm)?;

    match header.entry_type {
        EntryType::File => decompress_file(decoder, &header, &args)?,
        EntryType::Archive => decompress_archive(decoder, &header, &args)?,
    }

    Ok(())
}

fn resolve_decrypt_key(args: &DecompressArgs, header: &Header) -> Result<crypto::Key> {
    let key_material = match &args.key_file {
        Some(path) => {
            fs::read(path).with_context(|| format!("failed to read key file {}", path.display()))?
        }
        None => crypto::prompt_existing_password()?.into_bytes(),
    };
    crypto::derive_key(&key_material, &header.salt)
}

/// A directory that exists (or has an existing ancestor) to check available
/// disk space against, before any output is actually written.
fn bomb_check_dir(header: &Header, args: &DecompressArgs) -> Result<PathBuf> {
    match header.entry_type {
        EntryType::File => {
            let dest_path = resolve_file_output(&args.output, &header.name)?;
            Ok(dest_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from(".")))
        }
        EntryType::Archive => Ok(args.output.clone().unwrap_or(env::current_dir()?)),
    }
}

fn decompress_file<R: io::Read>(
    mut decoder: codec::Decoder<R>,
    header: &Header,
    args: &DecompressArgs,
) -> Result<()> {
    let dest_path = resolve_file_output(&args.output, &header.name)?;
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut out_file = File::create(&dest_path)
        .with_context(|| format!("failed to create {}", dest_path.display()))?;

    if args.verify {
        let mut hashing = HashingWriter::new(out_file);
        io::copy(&mut decoder, &mut hashing)
            .with_context(|| format!("failed to decompress into {}", dest_path.display()))?;
        let (_, size, checksum) = hashing.into_inner_with_stats();
        verify_checksum(header, size, checksum)?;
        println!("✔ verified checksum ({})", hex_encode(&checksum));
    } else {
        io::copy(&mut decoder, &mut out_file)
            .with_context(|| format!("failed to decompress into {}", dest_path.display()))?;
    }

    println!("✔ Decompressed -> {}", dest_path.display());
    if args.verbose {
        println!("  size: {}", human_bytes(header.original_size));
    }
    Ok(())
}

fn decompress_archive<R: io::Read>(
    mut decoder: codec::Decoder<R>,
    header: &Header,
    args: &DecompressArgs,
) -> Result<()> {
    let output_base = args.output.clone().unwrap_or(env::current_dir()?);
    fs::create_dir_all(&output_base)
        .with_context(|| format!("failed to create {}", output_base.display()))?;
    let target_dir = output_base.join(&header.name);

    // Streaming, no verify/dedup: unpack straight from the decoder.
    if !args.verify && !header.dedup {
        fs::create_dir_all(&target_dir)
            .with_context(|| format!("failed to create {}", target_dir.display()))?;
        tar::Archive::new(&mut decoder)
            .unpack(&target_dir)
            .with_context(|| format!("failed to unpack archive into {}", target_dir.display()))?;
        println!("✔ Decompressed -> {}", target_dir.display());
        return Ok(());
    }

    // Otherwise materialize the decompressed tar stream to a temp file
    // first, either to checksum it, to unpack it into a dedup staging
    // area, or both.
    let tmp_tar_path = sibling_temp_path(&target_dir);
    let materialize_result = (|| -> Result<()> {
        let tmp_file = File::create(&tmp_tar_path)
            .with_context(|| format!("failed to create {}", tmp_tar_path.display()))?;
        if args.verify {
            let mut hashing = HashingWriter::new(tmp_file);
            io::copy(&mut decoder, &mut hashing).context("failed to decompress archive")?;
            let (_, size, checksum) = hashing.into_inner_with_stats();
            verify_checksum(header, size, checksum)?;
            println!("✔ verified checksum ({})", hex_encode(&checksum));
        } else {
            let mut tmp_file = tmp_file;
            io::copy(&mut decoder, &mut tmp_file).context("failed to decompress archive")?;
        }
        Ok(())
    })();
    if materialize_result.is_err() {
        let _ = fs::remove_file(&tmp_tar_path);
        materialize_result?;
    }

    let unpack_result = (|| -> Result<()> {
        let tar_file = File::open(&tmp_tar_path)
            .with_context(|| format!("failed to reopen {}", tmp_tar_path.display()))?;
        let staging_dir = sibling_staging_path(&target_dir);
        dedup::unpack_tar(tar_file, header.dedup, &staging_dir, &target_dir)
    })();
    let _ = fs::remove_file(&tmp_tar_path);
    unpack_result?;

    println!("✔ Decompressed -> {}", target_dir.display());
    Ok(())
}

fn verify_checksum(header: &Header, size: u64, checksum: [u8; 32]) -> Result<()> {
    if size != header.original_size || checksum != header.checksum {
        bail!(
            "checksum verification FAILED: expected sha256 {} ({} bytes), got {} ({} bytes)",
            hex_encode(&header.checksum),
            header.original_size,
            hex_encode(&checksum),
            size
        );
    }
    Ok(())
}

fn resolve_file_output(output: &Option<PathBuf>, original_name: &str) -> Result<PathBuf> {
    match output {
        None => Ok(env::current_dir()?.join(original_name)),
        Some(p) if p.is_dir() => Ok(p.join(original_name)),
        Some(p) => Ok(p.clone()),
    }
}

fn sibling_temp_path(target_dir: &Path) -> PathBuf {
    let mut s = target_dir.as_os_str().to_os_string();
    s.push(".vaqum-tmp");
    PathBuf::from(s)
}

fn sibling_staging_path(target_dir: &Path) -> PathBuf {
    let mut s = target_dir.as_os_str().to_os_string();
    s.push(".vaqum-staging");
    PathBuf::from(s)
}
