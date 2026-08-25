use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::cli::CompressArgs;
use crate::codec;
use crate::crypto;
use crate::dedup;
use crate::format::{Algorithm, CountingWriter, EntryType, HashingWriter, Header};
use crate::util::human_bytes;

/// Result of streaming a source (file or directory) through the compressor.
struct PayloadStats {
    original_size: u64,
    compressed_size: u64,
    checksum: [u8; 32],
}

pub fn run(args: CompressArgs) -> Result<()> {
    let input = &args.path;
    if !input.exists() {
        bail!("'{}' does not exist", input.display());
    }
    let is_dir = input.is_dir();
    if is_dir && !args.recursive {
        bail!(
            "'{}' is a directory; pass -r/--recursive to compress it",
            input.display()
        );
    }
    if args.dedup && !is_dir {
        bail!("--dedup only applies to directories compressed with -r");
    }

    let algorithm = if args.max {
        Algorithm::Xz
    } else {
        Algorithm::Zstd
    };
    let entry_type = if is_dir {
        EntryType::Archive
    } else {
        EntryType::File
    };
    let threads = args.threads.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    });
    let name = input
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| input.display().to_string());

    let wants_encryption = args.encrypt || args.key_file.is_some();

    if args.dry_run {
        if wants_encryption {
            bail!("--dry-run doesn't support -e/--key-file; it only estimates compression");
        }
        let stats = build_payload(
            input,
            is_dir,
            args.dedup,
            algorithm,
            args.level,
            threads,
            io::sink(),
            args.verbose,
            None,
        )?;
        println!("(dry run) would compress '{}':", input.display());
        print_summary(&stats, algorithm, args.dedup);
        return Ok(());
    }

    let encryption = if wants_encryption {
        let key_material = match &args.key_file {
            Some(path) => fs::read(path)
                .with_context(|| format!("failed to read key file {}", path.display()))?,
            None => crypto::prompt_new_password()?.into_bytes(),
        };
        let salt = crypto::random_salt();
        let nonce_prefix = crypto::random_nonce_prefix();
        let key = crypto::derive_key(&key_material, &salt)?;
        Some((key, nonce_prefix, salt))
    } else {
        None
    };

    let output_path = args
        .output
        .clone()
        .unwrap_or_else(|| default_output_path(input));
    let tmp_path = sibling_temp_path(&output_path);

    let stats = {
        let tmp_file = File::create(&tmp_path)
            .with_context(|| format!("failed to create temporary file {}", tmp_path.display()))?;
        let encrypt_key = encryption
            .as_ref()
            .map(|(key, nonce_prefix, _)| (key, nonce_prefix));
        let result = build_payload(
            input,
            is_dir,
            args.dedup,
            algorithm,
            args.level,
            threads,
            tmp_file,
            args.verbose,
            encrypt_key,
        );
        if result.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }
        result?
    };

    let (salt, nonce_prefix) = match &encryption {
        Some((_, nonce_prefix, salt)) => (*salt, *nonce_prefix),
        None => ([0u8; crypto::SALT_LEN], [0u8; crypto::NONCE_PREFIX_LEN]),
    };
    let header = Header {
        entry_type,
        algorithm,
        dedup: args.dedup,
        encrypted: wants_encryption,
        original_size: stats.original_size,
        checksum: stats.checksum,
        salt,
        nonce_prefix,
        name,
    };

    let write_result = write_final_output(&output_path, &tmp_path, &header);
    let _ = fs::remove_file(&tmp_path);
    write_result?;

    let total_out = header.on_disk_len() + stats.compressed_size;
    let verb = if wants_encryption {
        "Compressed and encrypted"
    } else {
        "Compressed"
    };
    println!(
        "✔ {verb} '{}' -> '{}'",
        input.display(),
        output_path.display()
    );
    if wants_encryption {
        println!("  encryption:   ChaCha20-Poly1305, Argon2id KDF");
    }
    print_summary(&stats, algorithm, args.dedup);
    let ratio = if stats.original_size > 0 {
        stats.original_size as f64 / total_out.max(1) as f64
    } else {
        1.0
    };
    println!("  output size:  {} ({:.2}x)", human_bytes(total_out), ratio);

    Ok(())
}

fn write_final_output(output_path: &Path, tmp_path: &Path, header: &Header) -> Result<()> {
    let mut out = File::create(output_path)
        .with_context(|| format!("failed to create {}", output_path.display()))?;
    header.write(&mut out)?;
    let mut tmp_in =
        File::open(tmp_path).with_context(|| format!("failed to reopen {}", tmp_path.display()))?;
    io::copy(&mut tmp_in, &mut out)
        .with_context(|| format!("failed to write {}", output_path.display()))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_payload<W: Write>(
    input: &Path,
    is_dir: bool,
    dedup_enabled: bool,
    algorithm: Algorithm,
    level: u8,
    threads: usize,
    dest: W,
    verbose: bool,
    encrypt_key: Option<(&crypto::Key, &crypto::NoncePrefix)>,
) -> Result<PayloadStats> {
    let counting = CountingWriter::new(dest);
    let encrypt_writer = crypto::EncryptWriter::new(counting, encrypt_key);
    let encoder = codec::Encoder::new(encrypt_writer, algorithm, level, threads as u32)?;
    let hashing = HashingWriter::new(encoder);

    let hashing = if is_dir {
        let mut builder = tar::Builder::new(hashing);
        if dedup_enabled {
            dedup::write_dedup_tar(input, threads, &mut builder, |path, size, is_dup| {
                if verbose {
                    let tag = if is_dup { "dup " } else { "new " };
                    println!("  {tag}{path}  ({})", human_bytes(size));
                }
            })?;
        } else {
            append_directory_tar(&mut builder, input, verbose)?;
        }
        builder
            .into_inner()
            .context("failed to finalize tar stream")?
    } else {
        let mut hashing = hashing;
        let mut src =
            File::open(input).with_context(|| format!("failed to open {}", input.display()))?;
        io::copy(&mut src, &mut hashing)
            .with_context(|| format!("failed to read {}", input.display()))?;
        hashing
    };

    let (encoder, original_size, checksum) = hashing.into_inner_with_stats();
    let encrypt_writer = encoder
        .finish()
        .context("failed to finalize compressed stream")?;
    let counting = encrypt_writer
        .finish()
        .context("failed to finalize encrypted stream")?;
    let compressed_size = counting.count();

    Ok(PayloadStats {
        original_size,
        compressed_size,
        checksum,
    })
}

/// Walk `root` and append every directory and file into `builder`,
/// preserving relative paths and metadata, with optional per-file logging.
fn append_directory_tar<W: Write>(
    builder: &mut tar::Builder<W>,
    root: &Path,
    verbose: bool,
) -> Result<()> {
    for entry in walkdir::WalkDir::new(root).min_depth(1).sort_by_file_name() {
        let entry = entry.with_context(|| format!("failed to walk {}", root.display()))?;
        let rel = entry
            .path()
            .strip_prefix(root)
            .expect("walkdir entries are under root");
        let rel_str = crate::util::to_posix_path(rel);

        if entry.file_type().is_dir() {
            builder
                .append_dir(&rel_str, entry.path())
                .with_context(|| format!("failed to archive directory {rel_str}"))?;
        } else if entry.file_type().is_file() {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            builder
                .append_path_with_name(entry.path(), &rel_str)
                .with_context(|| format!("failed to archive {rel_str}"))?;
            if verbose {
                println!("  {rel_str}  ({})", human_bytes(size));
            }
        }
        // Symlinks and other special files are skipped; tar can represent
        // them but round-tripping them safely is out of scope for now.
    }
    Ok(())
}

fn print_summary(stats: &PayloadStats, algorithm: Algorithm, dedup: bool) {
    let ratio = if stats.compressed_size > 0 {
        stats.original_size as f64 / stats.compressed_size as f64
    } else {
        1.0
    };
    println!(
        "  algorithm:    {}{}",
        algorithm.label(),
        if dedup { " + dedup" } else { "" }
    );
    println!("  original:     {}", human_bytes(stats.original_size));
    println!(
        "  compressed:   {} ({:.2}x)",
        human_bytes(stats.compressed_size),
        ratio
    );
}

fn default_output_path(input: &Path) -> PathBuf {
    let mut s = input.as_os_str().to_os_string();
    s.push(".vaqum");
    PathBuf::from(s)
}

fn sibling_temp_path(output_path: &Path) -> PathBuf {
    let mut s = output_path.as_os_str().to_os_string();
    s.push(".tmp");
    PathBuf::from(s)
}
