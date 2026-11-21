use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "vaqum",
    bin_name = "vaqum",
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
    /// Compress one or more files/directories into a .vqm archive
    Compress(CompressArgs),
    /// Decompress a .vqm archive (older .vaqum files work too)
    Decompress(DecompressArgs),
    /// List the files inside an archive without extracting it
    #[command(visible_alias = "ls")]
    List(ListArgs),
    /// Securely overwrite and delete a file or directory
    Shred(ShredArgs),
    /// Show stats about a .vqm archive (without fully decompressing it),
    /// or about a plain file or directory
    Info(InfoArgs),
    /// Compare two files, directories, or .vqm archives (any mix)
    Diff(DiffArgs),
    /// Find and report duplicate files in a directory tree
    Dedupe(DedupeArgs),
    /// Search a file, directory, or subtree for a file name or file content
    /// match
    Search(SearchArgs),
    /// Create your personal key (for archives sent to you, and for signing)
    Keygen(KeygenArgs),
    /// Print your public key, to share with people who send you archives
    Pubkey(PubkeyArgs),
    /// Sign an archive, so recipients can check who made it
    Sign(SignArgs),
    /// Check an archive's signature against the sender's public key
    Verify(VerifyArgs),
    /// Print (or install) a shell completion script
    Completions(CompletionsArgs),
    /// Manage persisted defaults and profiles (~/.config/vaqum/config.toml)
    Config(ConfigArgs),
}

#[derive(Parser)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub action: ConfigAction,
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Print the resolved config (file merged over built-in defaults)
    Show,
    /// Print the config file's location
    Path,
    /// Write a starter config file, with everything commented out
    Init,
}

#[derive(Parser)]
pub struct CompletionsArgs {
    /// Shell to generate completions for (auto-detected from $SHELL with
    /// --install if omitted)
    #[arg(required_unless_present = "install")]
    pub shell: Option<clap_complete::Shell>,

    /// Write the script to this shell's standard user completions
    /// directory instead of printing it to stdout
    #[arg(long)]
    pub install: bool,
}

#[derive(Parser)]
pub struct CompressArgs {
    /// File(s) and/or directory(-ies) to compress into one archive. Use
    /// `-` to read a single file's contents from stdin
    #[arg(required = true, num_args = 1..)]
    pub paths: Vec<PathBuf>,

    /// Output path (default: <input>.vqm; required when compressing
    /// more than one path). Use `-` to write the archive to stdout (the
    /// default when the input is `-`)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Compression level, 1-22 (zstd scale); default 9, unless overridden
    /// by a config default/profile
    #[arg(short = 'l', long, value_parser = clap::value_parser!(u8).range(1..=22))]
    pub level: Option<u8>,

    /// Use LZMA/xz max-compression mode instead of zstd (slower, smaller)
    #[arg(long)]
    pub max: bool,

    /// Apply a named profile from the config file (see `vaqum config`)
    #[arg(long)]
    pub profile: Option<String>,

    /// Number of threads to use (default: all cores). Each uses about
    /// 50 MB of memory at the default level, so this also caps memory
    #[arg(short, long)]
    pub threads: Option<usize>,

    /// Compress a directory recursively
    #[arg(short, long)]
    pub recursive: bool,

    /// Enable deduplication across files in a directory
    #[arg(long)]
    pub dedup: bool,

    /// Skip paths matching this name/pattern when compressing a directory
    /// (repeatable). End it with `/` to exclude a directory (and its
    /// contents); without, it excludes a file. Also read from a
    /// `.vaqumignore` file at each directory's root
    #[arg(long = "exclude", value_name = "PATTERN")]
    pub exclude: Vec<String>,

    /// Show estimated ratio without writing output
    #[arg(long)]
    pub dry_run: bool,

    /// Show per-file stats
    #[arg(short, long)]
    pub verbose: bool,

    /// Encrypt the output (ChaCha20-Poly1305, prompts for a password)
    #[arg(short, long, conflicts_with = "key_file")]
    pub encrypt: bool,

    /// Encrypt the output using this file's bytes as the key material,
    /// instead of a password
    #[arg(long)]
    pub key_file: Option<PathBuf>,

    /// Encrypt for a person instead of with a password: their `vqm1…`
    /// public key, or a file containing it (repeatable; each recipient can
    /// open the archive with their own key)
    #[arg(long = "to", value_name = "PUBKEY", conflicts_with_all = ["encrypt", "key_file"])]
    pub to: Vec<String>,

    /// Sign the archive with your key (writes <archive>.minisig). Uses your
    /// default identity unless a path is given
    #[arg(long, value_name = "IDENTITY", num_args = 0..=1, require_equals = true)]
    pub sign: Option<Option<PathBuf>>,

    /// Suppress the progress bar (also auto-suppressed when not attached
    /// to a terminal, e.g. piped or redirected)
    #[arg(short, long)]
    pub quiet: bool,

    /// Skip the leak check (the warning about credentials such as .env
    /// files, private keys or API tokens in what you're compressing)
    #[arg(long)]
    pub no_secret_check: bool,
}

#[derive(Parser)]
pub struct DecompressArgs {
    /// .vqm archive to decompress (`-` reads it from stdin)
    pub path: PathBuf,

    /// Output location (default: current dir). Use `-` to write to stdout:
    /// a single-file archive's contents, or a directory archive's raw tar
    /// stream (e.g. `vaqum decompress a.vqm -o - | tar x`)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Extract only matching paths (repeatable). A directory extracts
    /// everything inside it. Wildcards work like .gitignore: `*.rs` matches
    /// at any depth, `docs/**/*.md` matches the path. Paths are as
    /// `vaqum list` shows them
    #[arg(long = "only", value_name = "PATH")]
    pub only: Vec<String>,

    /// Show extra output (e.g. the decompressed size)
    #[arg(short, long)]
    pub verbose: bool,

    /// Checksum-verify output matches original hash
    #[arg(long)]
    pub verify: bool,

    /// Decrypt using this file's bytes as the key material, instead of a
    /// password prompt
    #[arg(long)]
    pub key_file: Option<PathBuf>,

    /// Your identity, for archives encrypted for you with --to (default:
    /// the one `vaqum keygen` saved)
    #[arg(short = 'i', long, value_name = "IDENTITY")]
    pub identity: Option<PathBuf>,

    /// Check the archive's signature against this sender's public key
    /// before extracting anything; refuse if it doesn't match
    #[arg(long, value_name = "PUBKEY")]
    pub from: Option<String>,

    /// Abort if the archive's compression ratio exceeds n:1
    #[arg(long, default_value_t = crate::bomb::DEFAULT_MAX_RATIO)]
    pub max_ratio: f64,

    /// Bypass the archive bomb ratio/disk-space safety check
    #[arg(long)]
    pub force: bool,

    /// Suppress the progress bar (also auto-suppressed when not attached
    /// to a terminal, e.g. piped or redirected)
    #[arg(short, long)]
    pub quiet: bool,
}

#[derive(Parser)]
#[command(
    after_help = "Note: multi-pass overwrite is best-effort, not forensic-grade. SSDs \
(wear-leveling, TRIM) and copy-on-write filesystems (APFS, btrfs, ZFS) can keep \
copies of the data in places the overwrite never touches, as can backups and \
snapshots. Full-disk encryption is the reliable protection."
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

    /// Suppress the overwrite progress bar (also auto-suppressed when not
    /// attached to a terminal, e.g. piped or redirected)
    #[arg(short, long)]
    pub quiet: bool,
}

#[derive(Parser)]
pub struct ListArgs {
    /// .vqm, zip, or tar.gz archive to list (`-` reads a .vqm from stdin)
    pub path: PathBuf,

    /// Decrypt using this file's bytes as the key material, instead of a
    /// password prompt
    #[arg(long)]
    pub key_file: Option<PathBuf>,

    /// Your identity, for archives encrypted for you with --to
    #[arg(short = 'i', long, value_name = "IDENTITY")]
    pub identity: Option<PathBuf>,

    /// Print the listing as JSON
    #[arg(long)]
    pub json: bool,

    /// Suppress the progress bar
    #[arg(short, long)]
    pub quiet: bool,
}

#[derive(Parser)]
pub struct KeygenArgs {
    /// Where to save the identity (default: ~/.config/vaqum/identity)
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

#[derive(Parser)]
pub struct PubkeyArgs {
    /// Identity to show the public key of (default: your saved one)
    #[arg(short = 'i', long)]
    pub identity: Option<PathBuf>,
}

#[derive(Parser)]
pub struct SignArgs {
    /// Archive (or any file) to sign
    pub path: PathBuf,

    /// Identity to sign with (default: your saved one)
    #[arg(short = 'i', long)]
    pub identity: Option<PathBuf>,
}

#[derive(Parser)]
pub struct VerifyArgs {
    /// Archive (or any file) to check
    pub path: PathBuf,

    /// The sender's `vqm1…` public key, or a file containing it
    #[arg(long, value_name = "PUBKEY")]
    pub from: String,

    /// Signature file (default: <path>.minisig)
    #[arg(long)]
    pub signature: Option<PathBuf>,
}

#[derive(Parser)]
pub struct InfoArgs {
    /// .vqm archive, plain file, or directory to inspect
    pub path: PathBuf,
}

#[derive(Parser)]
#[command(
    after_help = "Exit codes (like the classic `diff`): 0 = identical, 1 = differences \
found, 2 = trouble (e.g. one side doesn't exist, or a file is being compared \
against a directory)."
)]
pub struct DiffArgs {
    /// First file, directory, or .vqm archive
    pub a: PathBuf,

    /// Second file, directory, or .vqm archive
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
    /// directly, so edits save normally; a .vqm side is decompressed to
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

    /// Suppress the scanning progress bar (also auto-suppressed when not
    /// attached to a terminal, e.g. piped or redirected)
    #[arg(short, long)]
    pub quiet: bool,
}

#[derive(Parser)]
#[command(
    after_help = "Exit codes (like grep): 0 = at least one match, 1 = no matches, 2 = trouble.\n\
Each result line is tagged so it's unambiguous which kind of match it is:\n  \
name     <path>                  — the file/directory name itself matched\n  \
content  <path>:<line>: <text>   — that line's content matched, inside <path>"
)]
pub struct SearchArgs {
    /// Text (or regex, with -E) to search for
    pub pattern: String,

    /// File or directory to search (default: current directory; searched
    /// recursively when it's a directory)
    pub path: Option<PathBuf>,

    /// Case-insensitive matching
    #[arg(short, long)]
    pub ignore_case: bool,

    /// Treat <pattern> as a regular expression instead of a literal
    /// substring
    #[arg(short = 'E', long)]
    pub regex: bool,

    /// Only match file/directory names, not file content
    #[arg(long, conflicts_with = "content_only")]
    pub names_only: bool,

    /// Only match file content, not file/directory names
    #[arg(long, conflicts_with = "names_only")]
    pub content_only: bool,

    /// Number of threads to use for scanning file content (default: all
    /// cores)
    #[arg(short, long)]
    pub threads: Option<usize>,
}
