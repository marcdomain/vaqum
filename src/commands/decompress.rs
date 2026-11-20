use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::{self, IsTerminal, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::archive::{self, Selectors};
use crate::bomb;
use crate::cli::DecompressArgs;
use crate::dedup::{self, MANIFEST_ENTRY, Manifest, OBJECTS_DIR};
use crate::extract::{self, Extracted};
use crate::foreign::{self, ForeignFormat};
use crate::format::{self, EntryType, HashingReader, HashingWriter, Header};
use crate::origin::Origin;
use crate::progress::{self, ProgressReader};
use crate::risk;
use crate::util::{
    AtomicFile, Out, hex_encode, human_bytes, is_stdio, set_unix_mode, to_posix_path,
};

pub fn run(args: DecompressArgs) -> Result<()> {
    if is_stdio(&args.path) || format::is_vaqum_file(&args.path)? {
        return run_vaqum(args);
    }
    if let Some(fmt) = foreign::detect(&args.path)? {
        if !args.only.is_empty() {
            bail!("--only works with .vqm archives, not {}", fmt.label());
        }
        if args.output.as_deref().is_some_and(is_stdio) {
            bail!(
                "-o - (stdout) works with .vqm archives, not {}",
                fmt.label()
            );
        }
        return run_foreign(args, fmt);
    }
    bail!(
        "'{}' is not a .vqm, zip, or tar.gz archive",
        args.path.display()
    );
}

fn run_foreign(args: DecompressArgs, fmt: ForeignFormat) -> Result<()> {
    let stats = foreign::inspect(&args.path, fmt)?;
    let dest = args.output.clone().unwrap_or(env::current_dir()?);
    bomb::check(
        stats.uncompressed_size,
        stats.compressed_size,
        &dest,
        args.max_ratio,
        args.force,
    )?;
    let mut ext = Extracted::default();
    let result = foreign::extract(&args.path, fmt, &dest, &mut ext);
    finish_extraction(&args.path, &ext);
    result?;
    println!(
        "✔ Extracted {} file(s) from {} archive -> {}",
        stats.file_count,
        fmt.label(),
        dest.display()
    );
    if args.verbose {
        println!("  uncompressed: {}", human_bytes(stats.uncompressed_size));
        println!("  compressed:   {}", human_bytes(stats.compressed_size));
    }
    Ok(())
}

fn run_vaqum(args: DecompressArgs) -> Result<()> {
    let to_stdout = args.output.as_deref().is_some_and(is_stdio);
    let out = Out::new(to_stdout);
    let src = archive::open_vaqum(&args.path)?;
    let header = src.header;

    if to_stdout {
        if !args.only.is_empty() {
            bail!("--only can't be combined with -o - (stdout)");
        }
        if header.dedup {
            bail!(
                "-o - (stdout) isn't supported for --dedup archives; decompress to a directory instead"
            );
        }
        if io::stdout().is_terminal() {
            bail!(
                "refusing to write decompressed data to the terminal; redirect it or pass -o <dir>"
            );
        }
    }

    // Bomb protection: the ratio check needs the on-disk size (unknown on
    // stdin); the disk-space check only matters when writing to disk.
    if let Some(compressed_size) = src.compressed_size {
        bomb::check_ratio(
            header.original_size,
            compressed_size,
            args.max_ratio,
            args.force,
        )?;
    }
    if !to_stdout && !args.force {
        bomb::check_disk_space(header.original_size, &bomb_check_dir(&header, &args)?)?;
    }

    let payload = archive::decoded_payload(src.reader, &header, args.key_file.as_deref())?;
    let bar = progress::bar(header.original_size, args.quiet, "Decompressing");
    let decoder = ProgressReader::new(payload, bar.clone());

    let mut ext = Extracted::default();
    let result = (|| -> Result<()> {
        if to_stdout {
            write_to_stdout(decoder, &header, &args)
        } else if !args.only.is_empty() {
            extract_selected(decoder, &header, &args, &mut ext)
        } else {
            match header.entry_type {
                EntryType::File => decompress_file(decoder, &header, &args, &mut ext),
                EntryType::Archive => {
                    let output_base = args.output.clone().unwrap_or(env::current_dir()?);
                    let target_dir = output_base.join(&header.name);
                    decompress_tar(decoder, &header, &args, &target_dir, &mut ext)
                }
                EntryType::Bundle => {
                    let output_base = args.output.clone().unwrap_or(env::current_dir()?);
                    decompress_tar(decoder, &header, &args, &output_base, &mut ext)
                }
            }
        }
    })();
    // Clear the progress bar before printing anything, or clearing it
    // afterwards would erase the last message instead.
    bar.finish_and_clear();
    for msg in REPORT.with(|r| r.take()) {
        out.say(msg);
    }
    finish_extraction(&args.path, &ext);
    result
}

/// After extracting (even partway): carry the archive's "downloaded from
/// the internet" mark over to everything written, and show any warnings.
fn finish_extraction(archive: &Path, ext: &Extracted) {
    if !is_stdio(archive)
        && let Some(origin) = Origin::of(archive)
    {
        origin.apply(&ext.written);
    }
    risk::print(&ext.warnings);
}

/// How warnings show an entry's path: under the archive's name for a
/// directory archive, as stored otherwise.
fn shown_prefix(header: &Header) -> String {
    match header.entry_type {
        EntryType::Archive => format!("{}/", header.name),
        _ => String::new(),
    }
}

thread_local! {
    /// Status messages queued while the progress bar is on screen.
    static REPORT: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Like `println!`, but held until the progress bar is cleared.
macro_rules! report {
    ($($arg:tt)*) => {
        REPORT.with(|r| r.borrow_mut().push(format!($($arg)*)))
    };
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
        EntryType::Archive | EntryType::Bundle => {
            Ok(args.output.clone().unwrap_or(env::current_dir()?))
        }
    }
}

/// `-o -`: the file's bytes, or a directory archive's raw tar stream.
fn write_to_stdout<R: Read>(decoder: R, header: &Header, args: &DecompressArgs) -> Result<()> {
    let mut reader = HashingReader::new(decoder, args.verify);
    let mut stdout = io::stdout().lock();
    io::copy(&mut reader, &mut stdout).context("failed to write to stdout")?;
    stdout.flush().ok();
    if args.verify {
        let (size, checksum) = reader.stats();
        verify_checksum(header, size, checksum)?;
        report!("✔ verified checksum ({})", hex_encode(&checksum));
    }
    Ok(())
}

fn decompress_file<R: io::Read>(
    mut decoder: R,
    header: &Header,
    args: &DecompressArgs,
    ext: &mut Extracted,
) -> Result<()> {
    let dest_path = resolve_file_output(&args.output, &header.name)?;
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    // Written to a temp file and moved into place only once complete (and
    // verified, with --verify), so a failure never leaves a partial file.
    let mut out_file = AtomicFile::create(&dest_path)?;
    if args.verify {
        let mut hashing = HashingWriter::new(out_file.file());
        io::copy(&mut decoder, &mut hashing)
            .with_context(|| format!("failed to decompress into {}", dest_path.display()))?;
        let (_, size, checksum) = hashing.into_inner_with_stats();
        verify_checksum(header, size, checksum)?;
        report!("✔ verified checksum ({})", hex_encode(&checksum));
    } else {
        io::copy(&mut decoder, out_file.file())
            .with_context(|| format!("failed to decompress into {}", dest_path.display()))?;
    }
    out_file.commit()?;
    ext.add_file(&dest_path, &header.name, None);

    report!("✔ Decompressed -> {}", dest_path.display());
    if args.verbose {
        report!("  size: {}", human_bytes(header.original_size));
    }
    Ok(())
}

fn decompress_tar<R: io::Read>(
    mut decoder: R,
    header: &Header,
    args: &DecompressArgs,
    target_dir: &Path,
    ext: &mut Extracted,
) -> Result<()> {
    if let Some(parent) = target_dir.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    // Streaming, no verify/dedup: unpack straight from the decoder.
    if !args.verify && !header.dedup {
        extract::unpack_tar(
            &mut decoder,
            target_dir,
            &shown_prefix(header),
            &mut |_| true,
            ext,
        )?;
        report!("✔ Decompressed -> {}", target_dir.display());
        return Ok(());
    }

    // Otherwise materialize the decompressed tar stream to a temp file
    // first, either to checksum it, to unpack it into a dedup staging
    // area, or both.
    let tar_file = materialize(&mut decoder, header, args.verify, target_dir)?;
    let staging = staging_dir_for(target_dir)?;
    dedup::unpack_tar(
        tar_file,
        header.dedup,
        &staging.path().join("objects"),
        target_dir,
        &shown_prefix(header),
        ext,
    )?;

    report!("✔ Decompressed -> {}", target_dir.display());
    Ok(())
}

/// Write the decoded tar stream to an anonymous temp file (no name, so
/// nothing else can open or swap it; the OS deletes it automatically),
/// checksum-verifying it if asked. Returns it rewound to the start.
fn materialize<R: Read>(
    decoder: &mut R,
    header: &Header,
    verify: bool,
    near: &Path,
) -> Result<File> {
    let mut tmp_file = tempfile::tempfile_in(existing_parent(near))
        .context("failed to create a temporary file")?;
    if verify {
        let mut hashing = HashingWriter::new(&mut tmp_file);
        io::copy(decoder, &mut hashing).context("failed to decompress archive")?;
        let (_, size, checksum) = hashing.into_inner_with_stats();
        verify_checksum(header, size, checksum)?;
        report!("✔ verified checksum ({})", hex_encode(&checksum));
    } else {
        io::copy(decoder, &mut tmp_file).context("failed to decompress archive")?;
    }
    tmp_file.seek(SeekFrom::Start(0))?;
    Ok(tmp_file)
}

/// A private, randomly named staging directory next to `target_dir`,
/// removed automatically when dropped (including on errors).
fn staging_dir_for(target_dir: &Path) -> Result<tempfile::TempDir> {
    tempfile::Builder::new()
        .prefix(".vaqum-staging-")
        .tempdir_in(existing_parent(target_dir))
        .context("failed to create a staging directory")
}

/// The nearest existing directory at or above `path`'s parent.
fn existing_parent(path: &Path) -> PathBuf {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .and_then(|p| p.ancestors().find(|a| a.is_dir()))
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

// ----------------------------------------------------------------- --only

fn extract_selected<R: Read>(
    decoder: R,
    header: &Header,
    args: &DecompressArgs,
    ext: &mut Extracted,
) -> Result<()> {
    let mut selectors = Selectors::new(&args.only)?;
    let output_base = args.output.clone().unwrap_or(env::current_dir()?);

    if header.entry_type == EntryType::File {
        if !selectors.select(header, &header.name) {
            selectors.ensure_all_matched()?;
        }
        return decompress_file(decoder, header, args, ext);
    }

    let target_dir = match header.entry_type {
        EntryType::Archive => output_base.join(&header.name),
        _ => output_base,
    };
    fs::create_dir_all(&target_dir)
        .with_context(|| format!("failed to create {}", target_dir.display()))?;

    if header.dedup {
        extract_selected_dedup(decoder, header, args, &mut selectors, &target_dir, ext)?;
    } else {
        extract_selected_plain(decoder, header, args, &mut selectors, &target_dir, ext)?;
    }
    selectors.ensure_all_matched()?;
    let extracted = ext.files;

    report!(
        "✔ Extracted {extracted} file{} -> {}",
        if extracted == 1 { "" } else { "s" },
        target_dir.display()
    );
    Ok(())
}

/// Streams the tar once, unpacking only matching entries. The whole stream
/// is still read so `--verify` can check the archive's checksum.
fn extract_selected_plain<R: Read>(
    decoder: R,
    header: &Header,
    args: &DecompressArgs,
    selectors: &mut Selectors,
    target_dir: &Path,
    ext: &mut Extracted,
) -> Result<()> {
    let mut reader = HashingReader::new(decoder, args.verify);
    extract::unpack_tar(
        &mut reader,
        target_dir,
        &shown_prefix(header),
        &mut |rel| selectors.select(header, rel),
        ext,
    )?;
    if args.verify {
        io::copy(&mut reader, &mut io::sink()).context("failed to read archive")?;
        let (size, checksum) = reader.stats();
        verify_checksum(header, size, checksum)?;
        report!("✔ verified checksum ({})", hex_encode(&checksum));
    }
    Ok(())
}

/// Dedup archives keep the real paths in a manifest at the end of the
/// stream, so: materialize the tar, read the manifest, then pull only the
/// content objects the selected files need.
fn extract_selected_dedup<R: Read>(
    mut decoder: R,
    header: &Header,
    args: &DecompressArgs,
    selectors: &mut Selectors,
    target_dir: &Path,
    ext: &mut Extracted,
) -> Result<()> {
    let mut tar_file = materialize(&mut decoder, header, args.verify, target_dir)?;

    let manifest = read_manifest(&mut tar_file)?;
    tar_file.seek(SeekFrom::Start(0))?;
    for dir in &manifest.dirs {
        if selectors.select(header, dir) {
            let path = archive::safe_join(target_dir, dir)?;
            fs::create_dir_all(&path)?;
            ext.add_dir(&path);
        }
    }
    // hash -> files that need that content
    let mut wanted: HashMap<&str, Vec<&dedup::FileEntry>> = HashMap::new();
    for file in &manifest.files {
        if selectors.select(header, &file.path) {
            if !archive::is_object_hash(&file.hash) {
                bail!("corrupt dedup manifest: invalid object id {:?}", file.hash);
            }
            archive::safe_join(target_dir, &file.path)?;
            wanted.entry(file.hash.as_str()).or_default().push(file);
        }
    }

    let prefix = shown_prefix(header);
    let mut tar = tar::Archive::new(&mut tar_file);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let rel = to_posix_path(&entry.path()?);
        let Some(hash) = rel.strip_prefix(&format!("{OBJECTS_DIR}/")) else {
            continue;
        };
        let Some(files) = wanted.get(hash) else {
            continue;
        };
        let first = archive::safe_join(target_dir, &files[0].path)?;
        let mut f = File::create(&first)
            .with_context(|| format!("failed to create {}", first.display()))?;
        io::copy(&mut entry, &mut f)?;
        drop(f);
        set_unix_mode(&first, files[0].mode)?;
        ext.add_file(&first, &format!("{prefix}{}", files[0].path), files[0].mode);
        for dup in &files[1..] {
            let dest = archive::safe_join(target_dir, &dup.path)?;
            fs::copy(&first, &dest).with_context(|| format!("failed to restore {}", dup.path))?;
            set_unix_mode(&dest, dup.mode)?;
            ext.add_file(&dest, &format!("{prefix}{}", dup.path), dup.mode);
        }
    }
    Ok(())
}

fn read_manifest(tar_file: &mut File) -> Result<Manifest> {
    let mut tar = tar::Archive::new(tar_file);
    for entry in tar.entries()? {
        let mut entry = entry?;
        if archive::normalize(&to_posix_path(&entry.path()?)) == MANIFEST_ENTRY {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            return serde_json::from_slice(&bytes).context("failed to parse dedup manifest");
        }
    }
    bail!("dedup archive is missing its manifest")
}

// ---------------------------------------------------------------- helpers

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
