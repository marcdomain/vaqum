use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "vaqum",
    version,
    about = "Losslessly compress, decompress, and securely shred files.",
    long_about = None,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Compress a file or directory into a .vaqum archive
    Compress(CompressArgs),
    /// Decompress a .vaqum archive
    Decompress(DecompressArgs),
    /// Securely overwrite and delete a file or directory
    Shred(ShredArgs),
    /// Show stats about a .vaqum archive without fully decompressing it
    Info(InfoArgs),
    /// Compare two files, directories, or .vaqum archives (any mix)
    Diff(DiffArgs),
    /// Find and report duplicate files in a directory tree
    Dedupe(DedupeArgs),
}

#[derive(Parser)]
pub struct CompressArgs {
    /// File or directory to compress
    pub path: PathBuf,

    /// Output path (default: <input>.vaqum)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Compression level, 1-22 (zstd scale)
    #[arg(short = 'l', long, default_value_t = 19, value_parser = clap::value_parser!(u8).range(1..=22))]
    pub level: u8,

    /// Use LZMA/xz max-compression mode instead of zstd (slower, smaller)
    #[arg(long)]
    pub max: bool,

    /// Number of threads to use (default: all cores)
    #[arg(short, long)]
    pub threads: Option<usize>,

    /// Compress a directory recursively
    #[arg(short, long)]
    pub recursive: bool,

    /// Enable deduplication across files in a directory
    #[arg(long)]
    pub dedup: bool,

    /// Show estimated ratio without writing output
    #[arg(long)]
    pub dry_run: bool,

    /// Show per-file stats
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Parser)]
pub struct DecompressArgs {
    /// .vaqum file to decompress
    pub path: PathBuf,

    /// Output location (default: current dir)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Show progress
    #[arg(short, long)]
    pub verbose: bool,

    /// Checksum-verify output matches original hash
    #[arg(long)]
    pub verify: bool,
}

#[derive(Parser)]
#[command(
    after_help = "Note: multi-pass overwrite is best-effort, not forensic-grade. On SSDs, \
wear-leveling and TRIM mean the drive can retain copies of data at physical \
locations the overwrite never touches."
)]
pub struct ShredArgs {
    /// File or directory to shred
    pub path: PathBuf,

    /// Shred a directory and its contents
    #[arg(short, long)]
    pub recursive: bool,

    /// Number of overwrite passes
    #[arg(short, long, default_value_t = 3)]
    pub passes: u32,

    /// Skip confirmation (for scripts/automation)
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Show what would be shredded, without doing it
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Parser)]
pub struct InfoArgs {
    /// .vaqum file to inspect
    pub path: PathBuf,
}

#[derive(Parser)]
#[command(
    after_help = "Exit codes (like the classic `diff`): 0 = identical, 1 = differences \
found, 2 = trouble (e.g. one side doesn't exist, or a file is being compared \
against a directory)."
)]
pub struct DiffArgs {
    /// First file, directory, or .vaqum archive
    pub a: PathBuf,

    /// Second file, directory, or .vaqum archive
    pub b: PathBuf,

    /// Show full unified diffs for every changed file, not just a summary
    /// (directories only; single-file diffs are always shown in full)
    #[arg(short, long)]
    pub verbose: bool,

    /// Write a self-contained HTML diff report to this file
    #[arg(long)]
    pub html: Option<PathBuf>,

    /// Open the HTML report in the default browser (writes one to a temp
    /// file first if --html wasn't given)
    #[arg(long)]
    pub open: bool,

    /// Open the diff in an editor's live, editable diff view instead of
    /// (or alongside) printing to the terminal. Runs `code --diff` by
    /// default (VS Code); override with $VAQUM_DIFF_EDITOR for another
    /// editor that supports the same `<editor> --diff <a> <b>` convention.
    /// For a plain file vs. a plain file this opens the real files
    /// directly, so edits save normally; a .vaqum side is decompressed to
    /// a scratch copy first (noted on screen) since there's nothing on
    /// disk to edit. For directories, opens one diff tab per modified
    /// text file (capped, to avoid flooding the editor).
    #[arg(short, long)]
    pub editor: bool,
}

#[derive(Parser)]
pub struct DedupeArgs {
    /// Directory tree to scan for duplicate files
    pub path: PathBuf,

    /// List every duplicate group's paths, not just the summary
    #[arg(short, long)]
    pub verbose: bool,

    /// Replace duplicates with hardlinks to the first occurrence, reclaiming
    /// disk space without deleting anything
    #[arg(long)]
    pub link: bool,

    /// With --link, show what would be linked without changing anything
    #[arg(long)]
    pub dry_run: bool,

    /// Number of threads to use for hashing (default: all cores)
    #[arg(short, long)]
    pub threads: Option<usize>,
}
