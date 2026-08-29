use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::cli::CompressArgs;
use crate::codec;
use crate::config::Config;
use crate::crypto;
use crate::dedup;
use crate::exclude::ExcludeSet;
use crate::format::{Algorithm, CountingWriter, EntryType, HashingWriter, Header};
use crate::progress::{self, ProgressWriter};
use crate::util::human_bytes;

/// Result of streaming a source (file, directory, or multi-path bundle)
/// through the compressor.
struct PayloadStats {
    original_size: u64,
    compressed_size: u64,
    checksum: [u8; 32],
}

/// A trailing separator (`node_modules/`) means the same thing as without
/// one — strip it so path handling downstream never has to special-case it.
fn normalize_path(path: &Path) -> PathBuf {
    path.components().collect()
}

pub fn run(args: CompressArgs) -> Result<()> {
    let paths: Vec<PathBuf> = args.paths.iter().map(|p| normalize_path(p)).collect();
    let paths = &paths;
    for path in paths {
        if !path.exists() {
            bail!("'{}' does not exist", path.display());
        }
    }
    if let Some(dir) = paths.iter().find(|p| p.is_dir())
        && !args.recursive
    {
        bail!(
            "'{}' is a directory; pass -r/--recursive to compress it",
            dir.display()
        );
    }
    if paths.len() > 1 {
        let mut seen = std::collections::HashSet::new();
        for path in paths {
            let basename = basename_of(path);
            if !seen.insert(basename.clone()) {
                bail!(
                    "multiple inputs share the name '{basename}'; rename one or compress them separately"
                );
            }
        }
    }
    let is_single_dir = paths.len() == 1 && paths[0].is_dir();

    let config = Config::load()?;
    let resolved = config.resolve_compress(args.profile.as_deref())?;
    let level = args.level.or(resolved.level).unwrap_or(9);
    let use_max = args.max || resolved.max.unwrap_or(false);
    let dedup_enabled = args.dedup || resolved.dedup.unwrap_or(false);
    let threads_opt = args.threads.or(resolved.threads);
    let mut exclude_patterns = args.exclude.clone();
    exclude_patterns.extend(resolved.exclude);
    let wants_encryption =
        args.encrypt || args.key_file.is_some() || resolved.encrypt.unwrap_or(false);

    if dedup_enabled && !is_single_dir {
        bail!("--dedup only applies to a single directory compressed with -r");
    }

    let algorithm = if use_max {
        Algorithm::Xz
    } else {
        Algorithm::Zstd
    };
    let entry_type = match paths.as_slice() {
        [single] if single.is_dir() => EntryType::Archive,
        [_single] => EntryType::File,
        _ => EntryType::Bundle,
    };
    let threads = threads_opt.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    });
    let name = describe_paths(paths);

    let total_bytes = total_input_bytes(paths, &exclude_patterns)?;

    if args.dry_run {
        if wants_encryption {
            bail!("--dry-run doesn't support -e/--key-file; it only estimates compression");
        }
        let bar = progress::bar(total_bytes, args.quiet, "Compressing");
        let stats = build_payload(
            paths,
            dedup_enabled,
            &exclude_patterns,
            algorithm,
            level,
            threads,
            io::sink(),
            args.verbose,
            None,
            bar.clone(),
        )?;
        bar.finish_and_clear();
        println!("(dry run) would compress '{name}':");
        print_summary(&stats, algorithm, dedup_enabled);
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

    let output_path = match &args.output {
        Some(path) => path.clone(),
        None if paths.len() == 1 => default_output_path(&paths[0]),
        None => {
            bail!("compressing multiple paths requires -o/--output to name the resulting archive")
        }
    };
    let tmp_path = sibling_temp_path(&output_path);

    let bar = progress::bar(total_bytes, args.quiet, "Compressing");
    let stats = {
        let tmp_file = File::create(&tmp_path)
            .with_context(|| format!("failed to create temporary file {}", tmp_path.display()))?;
        let encrypt_key = encryption
            .as_ref()
            .map(|(key, nonce_prefix, _)| (key, nonce_prefix));
        let result = build_payload(
            paths,
            dedup_enabled,
            &exclude_patterns,
            algorithm,
            level,
            threads,
            tmp_file,
            args.verbose,
            encrypt_key,
            bar.clone(),
        );
        if result.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }
        result?
    };
    bar.finish_and_clear();

    let (salt, nonce_prefix) = match &encryption {
        Some((_, nonce_prefix, salt)) => (*salt, *nonce_prefix),
        None => ([0u8; crypto::SALT_LEN], [0u8; crypto::NONCE_PREFIX_LEN]),
    };
    let header = Header {
        entry_type,
        algorithm,
        dedup: dedup_enabled,
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
    println!("✔ {verb} '{}' -> '{}'", header.name, output_path.display());
    if wants_encryption {
        println!("  encryption:   ChaCha20-Poly1305, Argon2id KDF");
    }
    print_summary(&stats, algorithm, dedup_enabled);
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
    paths: &[PathBuf],
    dedup_enabled: bool,
    exclude_patterns: &[String],
    algorithm: Algorithm,
    level: u8,
    threads: usize,
    dest: W,
    verbose: bool,
    encrypt_key: Option<(&crypto::Key, &crypto::NoncePrefix)>,
    bar: indicatif::ProgressBar,
) -> Result<PayloadStats> {
    let counting = CountingWriter::new(dest);
    let encrypt_writer = crypto::EncryptWriter::new(counting, encrypt_key);
    let encoder = codec::Encoder::new(encrypt_writer, algorithm, level, threads as u32)?;
    // Tracks pre-compression bytes flowing into the encoder.
    let progress_writer = ProgressWriter::new(encoder, bar);
    let hashing = HashingWriter::new(progress_writer);

    let hashing = match paths {
        [single] if !single.is_dir() => {
            let mut hashing = hashing;
            let mut src = File::open(single)
                .with_context(|| format!("failed to open {}", single.display()))?;
            io::copy(&mut src, &mut hashing)
                .with_context(|| format!("failed to read {}", single.display()))?;
            hashing
        }
        [single] => {
            let mut builder = tar::Builder::new(hashing);
            let exclude = ExcludeSet::build(single, exclude_patterns)?;
            if dedup_enabled {
                dedup::write_dedup_tar(
                    single,
                    threads,
                    Some(&exclude),
                    &mut builder,
                    |path, size, is_dup| {
                        if verbose {
                            let tag = if is_dup { "dup " } else { "new " };
                            println!("  {tag}{path}  ({})", human_bytes(size));
                        }
                    },
                )?;
            } else {
                append_directory_tar(&mut builder, single, "", Some(&exclude), verbose)?;
            }
            builder
                .into_inner()
                .context("failed to finalize tar stream")?
        }
        multiple => {
            let mut builder = tar::Builder::new(hashing);
            for path in multiple {
                append_bundle_entry(&mut builder, path, exclude_patterns, verbose)?;
            }
            builder
                .into_inner()
                .context("failed to finalize tar stream")?
        }
    };

    let (progress_writer, original_size, checksum) = hashing.into_inner_with_stats();
    let encrypt_writer = progress_writer
        .into_inner()
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

/// Add one top-level bundle entry (a file or a whole directory) under its
/// own basename, so multiple inputs can share one tar without colliding.
fn append_bundle_entry<W: Write>(
    builder: &mut tar::Builder<W>,
    path: &Path,
    exclude_patterns: &[String],
    verbose: bool,
) -> Result<()> {
    let basename = basename_of(path);

    if path.is_dir() {
        builder
            .append_dir(&basename, path)
            .with_context(|| format!("failed to archive directory {basename}"))?;
        let exclude = ExcludeSet::build(path, exclude_patterns)?;
        append_directory_tar(builder, path, &basename, Some(&exclude), verbose)
    } else {
        let size = path.metadata().map(|m| m.len()).unwrap_or(0);
        builder
            .append_path_with_name(path, &basename)
            .with_context(|| format!("failed to archive {basename}"))?;
        if verbose {
            println!("  {basename}  ({})", human_bytes(size));
        }
        Ok(())
    }
}

/// Walk `root` and append every directory and file into `builder`, with
/// paths prefixed by `prefix` (empty for a single-directory compress,
/// the entry's own basename when bundling multiple paths together).
fn append_directory_tar<W: Write>(
    builder: &mut tar::Builder<W>,
    root: &Path,
    prefix: &str,
    exclude: Option<&ExcludeSet>,
    verbose: bool,
) -> Result<()> {
    let walker = walkdir::WalkDir::new(root)
        .min_depth(1)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            let rel = e
                .path()
                .strip_prefix(root)
                .expect("walkdir entries are under root");
            !exclude.is_some_and(|ex| {
                ex.is_excluded(&crate::util::to_posix_path(rel), e.file_type().is_dir())
            })
        });

    for entry in walker {
        let entry = entry.with_context(|| format!("failed to walk {}", root.display()))?;
        let rel = entry
            .path()
            .strip_prefix(root)
            .expect("walkdir entries are under root");
        let rel_str = prefixed_posix_path(prefix, rel);

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

fn prefixed_posix_path(prefix: &str, rel: &Path) -> String {
    let rel_str = crate::util::to_posix_path(rel);
    if prefix.is_empty() {
        rel_str
    } else {
        format!("{prefix}/{rel_str}")
    }
}

fn total_input_bytes(paths: &[PathBuf], exclude_patterns: &[String]) -> Result<u64> {
    let mut total = 0u64;
    for path in paths {
        if path.is_dir() {
            let exclude = ExcludeSet::build(path, exclude_patterns)?;
            let walker = walkdir::WalkDir::new(path)
                .min_depth(1)
                .into_iter()
                .filter_entry(|e| {
                    let rel = e
                        .path()
                        .strip_prefix(path)
                        .expect("walkdir entries are under root");
                    !exclude.is_excluded(&crate::util::to_posix_path(rel), e.file_type().is_dir())
                });
            for entry in walker {
                let entry = entry.with_context(|| format!("failed to walk {}", path.display()))?;
                if entry.file_type().is_file() {
                    total += entry.metadata().map(|m| m.len()).unwrap_or(0);
                }
            }
        } else {
            total += path.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    Ok(total)
}

fn basename_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn describe_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| basename_of(p))
        .collect::<Vec<_>>()
        .join(", ")
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
