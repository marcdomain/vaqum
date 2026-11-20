use std::fs::{self, File};
use std::io::{self, IsTerminal, Seek, SeekFrom, Write};
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
use crate::util::{AtomicFile, Out, human_bytes, is_stdio};

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
    let from_stdin = paths.iter().any(|p| is_stdio(p));
    if from_stdin && paths.len() > 1 {
        bail!("`-` (stdin) can't be combined with other inputs");
    }
    let to_stdout =
        args.output.as_deref().is_some_and(is_stdio) || (from_stdin && args.output.is_none());
    let out = Out::new(to_stdout);
    if to_stdout && !args.dry_run && io::stdout().is_terminal() {
        bail!("refusing to write a binary archive to the terminal; redirect it or pass -o <file>");
    }
    for path in paths.iter().filter(|p| !is_stdio(p)) {
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
    let name = if from_stdin {
        stdin_entry_name(args.output.as_deref())
    } else {
        describe_paths(paths)
    };

    // Unknown up front for stdin; the progress bar shows bytes/speed only.
    let total_bytes = if from_stdin {
        0
    } else {
        total_input_bytes(paths, &exclude_patterns)?
    };

    if args.dry_run {
        if wants_encryption {
            bail!("--dry-run doesn't support -e/--key-file; it only estimates compression");
        }
        let bar = progress::bar(total_bytes, args.quiet || args.verbose, "Compressing");
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
            out,
        )?;
        bar.finish_and_clear();
        out.say(format!("(dry run) would compress '{name}':"));
        print_summary(&stats, algorithm, dedup_enabled, out);
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
        let key = crypto::derive_key(&key_material, &salt, true)?;
        Some((key, nonce_prefix, salt))
    } else {
        None
    };

    let output_path = match &args.output {
        _ if to_stdout => PathBuf::from("-"),
        Some(path) => path.clone(),
        None if paths.len() == 1 => default_output_path(&paths[0]),
        None => {
            bail!("compressing multiple paths requires -o/--output to name the resulting archive")
        }
    };
    let encrypt_key = encryption
        .as_ref()
        .map(|(key, nonce_prefix, _)| (key, nonce_prefix));

    // The header (which holds the payload's size and checksum) comes first
    // in the file, so the payload is staged in an anonymous temp file (it
    // has no name, so nothing else can open or replace it, and the OS
    // deletes it when vaqum exits) and copied in behind the header once
    // that's known. It's staged next to the output to stay on the same disk.
    let bar = progress::bar(total_bytes, args.quiet || args.verbose, "Compressing");
    let mut staging = if to_stdout {
        tempfile::tempfile()
    } else {
        tempfile::tempfile_in(output_dir(&output_path))
    }
    .context("failed to create a temporary file")?;
    let stats = {
        let result = build_payload(
            paths,
            dedup_enabled,
            &exclude_patterns,
            algorithm,
            level,
            threads,
            &staging,
            args.verbose,
            encrypt_key,
            bar.clone(),
            out,
        );
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
        strong_kdf: wants_encryption,
        original_size: stats.original_size,
        checksum: stats.checksum,
        salt,
        nonce_prefix,
        name,
    };

    staging.seek(SeekFrom::Start(0))?;
    if to_stdout {
        let mut stdout = io::stdout().lock();
        header.write(&mut stdout)?;
        io::copy(&mut staging, &mut stdout).context("failed to write archive to stdout")?;
        stdout
            .flush()
            .context("failed to write archive to stdout")?;
    } else {
        let mut out_file = AtomicFile::create(&output_path)?;
        header.write(out_file.file())?;
        io::copy(&mut staging, out_file.file())
            .with_context(|| format!("failed to write {}", output_path.display()))?;
        out_file.commit()?;
    }

    let total_out = header.on_disk_len() + stats.compressed_size;
    let verb = if wants_encryption {
        "Compressed and encrypted"
    } else {
        "Compressed"
    };
    let dest = if to_stdout {
        "stdout".to_string()
    } else {
        output_path.display().to_string()
    };
    out.say(format!("✔ {verb} '{}' -> '{dest}'", header.name));
    if wants_encryption {
        out.say("  encryption:   ChaCha20-Poly1305, Argon2id KDF");
    }
    print_summary(&stats, algorithm, dedup_enabled, out);
    let ratio = if stats.original_size > 0 {
        stats.original_size as f64 / total_out.max(1) as f64
    } else {
        1.0
    };
    out.say(format!(
        "  output size:  {} ({:.2}x)",
        human_bytes(total_out),
        ratio
    ));

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
    out: Out,
) -> Result<PayloadStats> {
    let counting = CountingWriter::new(dest);
    let encrypt_writer = crypto::EncryptWriter::new(counting, encrypt_key);
    let encoder = codec::Encoder::new(encrypt_writer, algorithm, level, threads as u32)?;
    // Tracks pre-compression bytes flowing into the encoder.
    let progress_writer = ProgressWriter::new(encoder, bar);
    let hashing = HashingWriter::new(progress_writer);

    let hashing = match paths {
        [single] if is_stdio(single) => {
            let mut hashing = hashing;
            io::copy(&mut io::stdin().lock(), &mut hashing).context("failed to read stdin")?;
            hashing
        }
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
                            out.say(format!("  {tag}{path}  ({})", human_bytes(size)));
                        }
                    },
                )?;
            } else {
                append_directory_tar(&mut builder, single, "", Some(&exclude), verbose, out)?;
            }
            builder
                .into_inner()
                .context("failed to finalize tar stream")?
        }
        multiple => {
            let mut builder = tar::Builder::new(hashing);
            for path in multiple {
                append_bundle_entry(&mut builder, path, exclude_patterns, verbose, out)?;
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
    out: Out,
) -> Result<()> {
    let basename = basename_of(path);

    if path.is_dir() {
        builder
            .append_dir(&basename, path)
            .with_context(|| format!("failed to archive directory {basename}"))?;
        let exclude = ExcludeSet::build(path, exclude_patterns)?;
        append_directory_tar(builder, path, &basename, Some(&exclude), verbose, out)
    } else {
        let size = path.metadata().map(|m| m.len()).unwrap_or(0);
        builder
            .append_path_with_name(path, &basename)
            .with_context(|| format!("failed to archive {basename}"))?;
        if verbose {
            out.say(format!("  {basename}  ({})", human_bytes(size)));
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
    out: Out,
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
                out.say(format!("  {rel_str}  ({})", human_bytes(size)));
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

fn print_summary(stats: &PayloadStats, algorithm: Algorithm, dedup: bool, out: Out) {
    let ratio = if stats.compressed_size > 0 {
        stats.original_size as f64 / stats.compressed_size as f64
    } else {
        1.0
    };
    out.say(format!(
        "  algorithm:    {}{}",
        algorithm.label(),
        if dedup { " + dedup" } else { "" }
    ));
    out.say(format!(
        "  original:     {}",
        human_bytes(stats.original_size)
    ));
    out.say(format!(
        "  compressed:   {} ({:.2}x)",
        human_bytes(stats.compressed_size),
        ratio
    ));
}

fn default_output_path(input: &Path) -> PathBuf {
    let mut s = input.as_os_str().to_os_string();
    s.push(".vqm");
    PathBuf::from(s)
}

fn output_dir(output_path: &Path) -> PathBuf {
    match output_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Name stored in the header for stdin input (used when the archive is
/// later decompressed to a file): the output's name minus `.vqm` (or the
/// older `.vaqum`), or "stdin".
fn stdin_entry_name(output: Option<&Path>) -> String {
    output
        .filter(|p| !is_stdio(p))
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .map(|n| {
            n.strip_suffix(".vqm")
                .or_else(|| n.strip_suffix(".vaqum"))
                .map(str::to_string)
                .unwrap_or(n)
        })
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "stdin".to_string())
}
