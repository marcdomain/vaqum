//! End-to-end tests that drive the real `vaqum` binary, the same way a
//! user would from a shell. Run with `cargo test`.

use std::fs;
use std::io::Write;
use std::sync::OnceLock;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// Empty, shared config dir so tests never see the real machine's
/// `~/.config/vaqum`. Config-loading tests override it per-test.
fn isolated_config_home() -> &'static std::path::Path {
    static DIR: OnceLock<TempDir> = OnceLock::new();
    DIR.get_or_init(|| TempDir::new().unwrap()).path()
}

fn vaqum() -> Command {
    let mut cmd = Command::cargo_bin("vaqum").expect("vaqum binary should build");
    cmd.env("XDG_CONFIG_HOME", isolated_config_home());
    cmd.env("APPDATA", isolated_config_home());
    cmd
}

/// Write a moderately large, non-trivially-compressible text file so
/// compression ratios in assertions are meaningful.
fn write_sample_text(path: &std::path::Path, repeats: usize) {
    let mut f = fs::File::create(path).unwrap();
    for i in 0..repeats {
        writeln!(f, "line {i}: the quick brown fox jumps over the lazy dog").unwrap();
    }
}

// ---------------------------------------------------------------------
// compress / decompress: single file
// ---------------------------------------------------------------------

#[test]
fn compress_decompress_single_file_round_trips_byte_for_byte() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("original.txt");
    write_sample_text(&input, 2000);

    vaqum()
        .args(["compress", input.to_str().unwrap()])
        .assert()
        .success();

    let archive = dir.path().join("original.txt.vqm");
    assert!(archive.exists(), "compressed archive should be created");

    let out_dir = dir.path().join("out");
    fs::create_dir_all(&out_dir).unwrap();
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
            "--verify",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("verified checksum"));

    let restored = out_dir.join("original.txt");
    assert_eq!(
        fs::read(&input).unwrap(),
        fs::read(&restored).unwrap(),
        "decompressed output must be byte-for-byte identical to the original"
    );
}

#[test]
fn compress_max_uses_xz_and_round_trips() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("original.txt");
    write_sample_text(&input, 2000);
    let archive = dir.path().join("out.vqm");

    vaqum()
        .args([
            "compress",
            input.to_str().unwrap(),
            "--max",
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .success();

    vaqum()
        .args(["info", archive.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("xz (LZMA)"));

    let out_dir = dir.path().join("out");
    fs::create_dir_all(&out_dir).unwrap();
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
            "--verify",
        ])
        .assert()
        .success();

    assert_eq!(
        fs::read(&input).unwrap(),
        fs::read(out_dir.join("original.txt")).unwrap()
    );
}

#[test]
fn dry_run_reports_stats_but_writes_no_output() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("original.txt");
    write_sample_text(&input, 500);

    vaqum()
        .args(["compress", input.to_str().unwrap(), "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("dry run"));

    assert!(
        !dir.path().join("original.txt.vqm").exists(),
        "--dry-run must not write an output file"
    );
}

// ---------------------------------------------------------------------
// progress bar: auto-suppressed when not a terminal, -q/--quiet accepted
// ---------------------------------------------------------------------

#[test]
fn compress_and_decompress_progress_bar_is_suppressed_when_piped() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("original.txt");
    write_sample_text(&input, 500);

    let archive = dir.path().join("original.txt.vqm");
    vaqum()
        .args(["compress", input.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("ETA").not())
        .stderr(predicate::str::contains("ETA").not());

    let out_dir = dir.path().join("out");
    fs::create_dir_all(&out_dir).unwrap();
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("ETA").not())
        .stderr(predicate::str::contains("ETA").not());
}

#[test]
fn compress_and_decompress_accept_quiet_flag() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("original.txt");
    write_sample_text(&input, 500);

    let archive = dir.path().join("original.txt.vqm");
    vaqum()
        .args(["compress", input.to_str().unwrap(), "-q"])
        .assert()
        .success();

    let out_dir = dir.path().join("out");
    fs::create_dir_all(&out_dir).unwrap();
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
            "--quiet",
        ])
        .assert()
        .success();
}

// ---------------------------------------------------------------------
// compress / decompress: directories, with and without --dedup
// ---------------------------------------------------------------------

fn build_sample_tree(root: &std::path::Path) {
    fs::create_dir_all(root.join("nested")).unwrap();
    write_sample_text(&root.join("a.txt"), 300);
    // Duplicate content under a different name, to exercise dedup.
    fs::copy(root.join("a.txt"), root.join("b.txt")).unwrap();
    write_sample_text(&root.join("nested/c.txt"), 50);
}

#[test]
fn compress_decompress_directory_round_trips() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src_tree");
    build_sample_tree(&src);

    let archive = dir.path().join("tree.vqm");
    vaqum()
        .args([
            "compress",
            src.to_str().unwrap(),
            "-r",
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .success();

    let out_dir = dir.path().join("out");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
            "--verify",
        ])
        .assert()
        .success();

    assert_trees_equal(&src, &out_dir.join("src_tree"));
}

#[test]
fn compress_decompress_directory_with_dedup_round_trips_and_shrinks() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src_tree");
    build_sample_tree(&src);

    let plain_archive = dir.path().join("plain.vqm");
    let dedup_archive = dir.path().join("dedup.vqm");

    vaqum()
        .args([
            "compress",
            src.to_str().unwrap(),
            "-r",
            "-o",
            plain_archive.to_str().unwrap(),
        ])
        .assert()
        .success();

    vaqum()
        .args([
            "compress",
            src.to_str().unwrap(),
            "-r",
            "--dedup",
            "-v",
            "-o",
            dedup_archive.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("dup b.txt"));

    // The dedup manifest records a.txt/b.txt as identical content, so the
    // *original* (pre-compression) payload dedup feeds the compressor is
    // smaller than the plain archive's — a guaranteed effect of skipping
    // the duplicate's raw bytes, independent of how well the compressor
    // itself would have deduplicated them within its own window.
    let plain_original = original_size_from_info(&plain_archive);
    let dedup_original = original_size_from_info(&dedup_archive);
    assert!(
        dedup_original < plain_original,
        "dedup's reported original size ({dedup_original}) should be smaller than plain's ({plain_original})"
    );

    let out_dir = dir.path().join("out");
    vaqum()
        .args([
            "decompress",
            dedup_archive.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
            "--verify",
        ])
        .assert()
        .success();

    assert_trees_equal(&src, &out_dir.join("src_tree"));
}

// ---------------------------------------------------------------------
// compress / decompress: multiple paths bundled into one archive
// ---------------------------------------------------------------------

#[test]
fn compress_multiple_files_bundles_without_requiring_recursive() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    write_sample_text(&a, 50);
    write_sample_text(&b, 30);

    let archive = dir.path().join("bundle.vqm");
    vaqum()
        .args([
            "compress",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .success();

    vaqum()
        .args(["info", archive.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("multi-path bundle"));

    let out_dir = dir.path().join("out");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
            "--verify",
        ])
        .assert()
        .success();

    assert_eq!(
        fs::read(&a).unwrap(),
        fs::read(out_dir.join("a.txt")).unwrap()
    );
    assert_eq!(
        fs::read(&b).unwrap(),
        fs::read(out_dir.join("b.txt")).unwrap()
    );
}

#[test]
fn compress_mixed_files_and_directory_requires_recursive_and_round_trips() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.txt");
    write_sample_text(&a, 20);
    let src = dir.path().join("src_tree");
    build_sample_tree(&src);

    let archive = dir.path().join("mixed.vqm");

    // A directory among the inputs still requires -r, same as a single
    // directory would.
    vaqum()
        .args([
            "compress",
            a.to_str().unwrap(),
            src.to_str().unwrap(),
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--recursive"));

    vaqum()
        .args([
            "compress",
            a.to_str().unwrap(),
            src.to_str().unwrap(),
            "-r",
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .success();

    let out_dir = dir.path().join("out");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
            "--verify",
        ])
        .assert()
        .success();

    assert_eq!(
        fs::read(&a).unwrap(),
        fs::read(out_dir.join("a.txt")).unwrap()
    );
    assert_trees_equal(&src, &out_dir.join("src_tree"));
}

#[test]
fn compress_multiple_paths_without_output_fails() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    write_sample_text(&a, 5);
    write_sample_text(&b, 5);

    vaqum()
        .args(["compress", a.to_str().unwrap(), b.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("-o/--output"));
}

#[test]
fn compress_multiple_paths_with_clashing_basenames_fails() {
    let dir = TempDir::new().unwrap();
    let dir_a = dir.path().join("dir_a");
    let dir_b = dir.path().join("dir_b");
    fs::create_dir_all(&dir_a).unwrap();
    fs::create_dir_all(&dir_b).unwrap();
    write_sample_text(&dir_a.join("same.txt"), 5);
    write_sample_text(&dir_b.join("same.txt"), 5);

    vaqum()
        .args([
            "compress",
            dir_a.join("same.txt").to_str().unwrap(),
            dir_b.join("same.txt").to_str().unwrap(),
            "-o",
            dir.path().join("out.vqm").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("share the name"));
}

#[test]
fn compress_multiple_paths_rejects_dedup() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.txt");
    let src = dir.path().join("src_tree");
    write_sample_text(&a, 5);
    build_sample_tree(&src);

    vaqum()
        .args([
            "compress",
            a.to_str().unwrap(),
            src.to_str().unwrap(),
            "-r",
            "--dedup",
            "-o",
            dir.path().join("out.vqm").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--dedup"));
}

// ---------------------------------------------------------------------
// compress --exclude / .vaqumignore
// ---------------------------------------------------------------------

fn build_excludable_tree(root: &std::path::Path) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
    write_sample_text(&root.join("src/main.rs"), 5);
    write_sample_text(&root.join("node_modules/pkg/index.js"), 5);
    write_sample_text(&root.join("debug.log"), 5);
    write_sample_text(&root.join("README.md"), 5);
}

#[test]
fn compress_exclude_flag_skips_matching_dir_and_file_pattern() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("proj");
    build_excludable_tree(&src);

    let archive = dir.path().join("out.vqm");
    vaqum()
        .args([
            "compress",
            src.to_str().unwrap(),
            "-r",
            "--exclude",
            "node_modules/",
            "--exclude",
            "*.log",
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .success();

    let out_dir = dir.path().join("out");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
        ])
        .assert()
        .success();

    let mut files = walk_relative(&out_dir.join("proj"));
    files.sort();
    assert_eq!(
        files,
        vec!["README.md".to_string(), "src/main.rs".to_string()]
    );
}

#[test]
fn compress_vaqumignore_file_is_auto_discovered() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("proj");
    build_excludable_tree(&src);
    fs::write(src.join(".vaqumignore"), "node_modules/\n*.log\n").unwrap();

    let archive = dir.path().join("out.vqm");
    vaqum()
        .args([
            "compress",
            src.to_str().unwrap(),
            "-r",
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .success();

    let out_dir = dir.path().join("out");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
        ])
        .assert()
        .success();

    let mut files = walk_relative(&out_dir.join("proj"));
    files.sort();
    assert_eq!(
        files,
        vec![
            ".vaqumignore".to_string(),
            "README.md".to_string(),
            "src/main.rs".to_string(),
        ]
    );
}

#[test]
fn compress_exclude_directory_pattern_does_not_match_a_same_named_file() {
    // "target/" (trailing slash) should exclude the *directory* named
    // "target" (and everything under it) but leave a plain file that
    // happens to share the name alone, elsewhere in the tree.
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("proj");
    fs::create_dir_all(src.join("build/target")).unwrap();
    write_sample_text(&src.join("build/target/output.bin"), 5);
    fs::create_dir_all(src.join("docs")).unwrap();
    write_sample_text(&src.join("docs/target"), 5);

    let archive = dir.path().join("out.vqm");
    vaqum()
        .args([
            "compress",
            src.to_str().unwrap(),
            "-r",
            "--exclude",
            "target/",
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .success();

    let out_dir = dir.path().join("out");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
        ])
        .assert()
        .success();

    let files = walk_relative(&out_dir.join("proj"));
    assert_eq!(files, vec!["docs/target".to_string()]);
}

#[test]
fn compress_trailing_slash_on_input_path_is_normalized() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("proj");
    build_excludable_tree(&src);

    let mut src_with_slash = src.to_str().unwrap().to_string();
    src_with_slash.push('/');

    // No -o given: the default output name must be "proj.vqm", not
    // corrupted into "proj/.vqm" by the trailing slash.
    vaqum()
        .current_dir(dir.path())
        .args(["compress", &src_with_slash, "-r"])
        .assert()
        .success();

    assert!(dir.path().join("proj.vqm").exists());
}

fn assert_trees_equal(a: &std::path::Path, b: &std::path::Path) {
    let mut a_files: Vec<_> = walk_relative(a);
    let mut b_files: Vec<_> = walk_relative(b);
    a_files.sort();
    b_files.sort();
    assert_eq!(a_files, b_files, "directory listings differ");

    for rel in a_files {
        assert_eq!(
            fs::read(a.join(&rel)).unwrap(),
            fs::read(b.join(&rel)).unwrap(),
            "content differs for {rel}"
        );
    }
}

/// Run `vaqum info` and parse the human-readable "original:" size back
/// into an approximate byte count, precise enough to compare orderings.
fn original_size_from_info(archive: &std::path::Path) -> f64 {
    let output = vaqum()
        .args(["info", archive.to_str().unwrap()])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .find(|l| l.trim_start().starts_with("original:"))
        .expect("info output should include an 'original:' line");
    let value = line.trim_start().trim_start_matches("original:").trim();
    let mut parts = value.split_whitespace();
    let number: f64 = parts.next().unwrap().parse().unwrap();
    let unit = parts.next().unwrap_or("B");
    let multiplier = match unit {
        "B" => 1.0,
        "kB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        other => panic!("unexpected size unit: {other}"),
    };
    number * multiplier
}

/// Relative file paths under `root`, always `/`-separated.
fn walk_relative(root: &std::path::Path) -> Vec<String> {
    walkdir::WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            e.path()
                .strip_prefix(root)
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect()
}

// ---------------------------------------------------------------------
// info
// ---------------------------------------------------------------------

#[test]
fn info_reports_algorithm_and_sizes() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("original.txt");
    write_sample_text(&input, 1000);
    let archive = dir.path().join("original.txt.vqm");

    vaqum()
        .args(["compress", input.to_str().unwrap()])
        .assert()
        .success();

    vaqum()
        .args(["info", archive.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("zstd"))
        .stdout(predicate::str::contains("single file"))
        .stdout(predicate::str::contains("checksum:"));
}

#[test]
fn info_on_a_plain_file_shows_size_and_checksum() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("plain.txt");
    fs::write(&input, b"hello world").unwrap();

    vaqum()
        .args(["info", input.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("type:         file"))
        .stdout(predicate::str::contains("11 B"))
        .stdout(predicate::str::contains("checksum:     sha256:"))
        .stdout(predicate::str::contains("modified:"));
}

#[test]
fn info_on_a_directory_shows_file_and_dir_counts() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("mydir");
    fs::create_dir_all(root.join("sub")).unwrap();
    fs::write(root.join("a.txt"), b"aa").unwrap();
    fs::write(root.join("sub/b.txt"), b"bb").unwrap();

    vaqum()
        .args(["info", root.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("type:         directory"))
        .stdout(predicate::str::contains("files:        2"))
        .stdout(predicate::str::contains("directories:  1"))
        .stdout(predicate::str::contains("modified:"));
}

#[test]
fn info_on_a_missing_path_fails() {
    vaqum()
        .args(["info", "/no/such/path/at/all"])
        .assert()
        .failure();
}

// ---------------------------------------------------------------------
// error paths
// ---------------------------------------------------------------------

#[test]
fn compress_directory_without_recursive_flag_fails() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src_tree");
    build_sample_tree(&src);

    vaqum()
        .args(["compress", src.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--recursive"));
}

#[test]
fn decompress_rejects_non_vaqum_file() {
    let dir = TempDir::new().unwrap();
    let bogus = dir.path().join("bogus.vqm");
    fs::write(&bogus, b"not a vaqum archive").unwrap();

    vaqum()
        .args(["decompress", bogus.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn dedup_flag_without_recursive_directory_fails() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("file.txt");
    write_sample_text(&input, 10);

    vaqum()
        .args(["compress", input.to_str().unwrap(), "--dedup"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--dedup"));
}

// ---------------------------------------------------------------------
// shred
// ---------------------------------------------------------------------

#[test]
fn shred_dry_run_leaves_file_untouched() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("secret.txt");
    fs::write(&target, b"sensitive data").unwrap();

    vaqum()
        .args(["shred", target.to_str().unwrap(), "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("dry run"));

    assert!(target.exists(), "--dry-run must not touch the file");
}

#[test]
fn shred_with_yes_flag_removes_file_without_prompting() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("secret.txt");
    fs::write(&target, b"sensitive data").unwrap();

    vaqum()
        .args(["shred", target.to_str().unwrap(), "-y"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Shredded"));

    assert!(!target.exists(), "file should be gone after shredding");
}

#[test]
fn shred_accepts_quiet_flag() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("secret.txt");
    fs::write(&target, b"sensitive data").unwrap();

    vaqum()
        .args(["shred", target.to_str().unwrap(), "-y", "-q"])
        .assert()
        .success();

    assert!(!target.exists(), "file should be gone after shredding");
}

#[test]
fn shred_aborts_when_confirmation_does_not_match() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("secret.txt");
    fs::write(&target, b"sensitive data").unwrap();

    vaqum()
        .args(["shred", target.to_str().unwrap()])
        .write_stdin("definitely-not-the-filename\n")
        .assert()
        .failure();

    assert!(target.exists(), "file must survive an aborted shred");
}

#[test]
fn shred_recursive_removes_directory_tree() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("to_shred");
    build_sample_tree(&src);

    vaqum()
        .args(["shred", src.to_str().unwrap(), "-r", "-y"])
        .assert()
        .success();

    assert!(!src.exists(), "directory tree should be fully removed");
}

// ---------------------------------------------------------------------
// diff
// ---------------------------------------------------------------------

#[test]
fn diff_identical_files_exits_zero() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    write_sample_text(&a, 20);
    fs::copy(&a, &b).unwrap();

    vaqum()
        .args(["diff", a.to_str().unwrap(), b.to_str().unwrap()])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("identical"));
}

#[test]
fn diff_different_text_files_exits_one_with_unified_diff() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    fs::write(&a, "line1\nline2\nline3\n").unwrap();
    fs::write(&b, "line1\nCHANGED\nline3\n").unwrap();

    vaqum()
        .args(["diff", a.to_str().unwrap(), b.to_str().unwrap()])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("-line2"))
        .stdout(predicate::str::contains("+CHANGED"));
}

#[test]
fn diff_binary_files_reports_differ_without_garbage_output() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.bin");
    let b = dir.path().join("b.bin");
    fs::write(&a, [0u8, 1, 2, 3, 255, 254]).unwrap();
    fs::write(&b, [0u8, 9, 9, 9, 255, 254]).unwrap();

    vaqum()
        .args(["diff", a.to_str().unwrap(), b.to_str().unwrap()])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Binary files"));
}

#[test]
fn diff_missing_path_exits_two() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.txt");
    fs::write(&a, "hi\n").unwrap();

    vaqum()
        .args(["diff", a.to_str().unwrap(), "does-not-exist"])
        .assert()
        .code(2);
}

#[test]
fn diff_file_against_directory_exits_two() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.txt");
    fs::write(&a, "hi\n").unwrap();
    let d = dir.path().join("a_dir");
    fs::create_dir(&d).unwrap();

    vaqum()
        .args(["diff", a.to_str().unwrap(), d.to_str().unwrap()])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("file against a directory"));
}

#[test]
fn diff_directories_reports_added_removed_modified() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();

    fs::write(a.join("same.txt"), "unchanged\n").unwrap();
    fs::write(b.join("same.txt"), "unchanged\n").unwrap();
    fs::write(a.join("changed.txt"), "old\n").unwrap();
    fs::write(b.join("changed.txt"), "new\n").unwrap();
    fs::write(a.join("removed.txt"), "gone\n").unwrap();
    fs::write(b.join("added.txt"), "new file\n").unwrap();

    vaqum()
        .args(["diff", a.to_str().unwrap(), b.to_str().unwrap(), "-v"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("removed.txt"))
        .stdout(predicate::str::contains("added.txt"))
        .stdout(predicate::str::contains("changed.txt"))
        .stdout(predicate::str::contains("-old"))
        .stdout(predicate::str::contains("+new"));
}

#[test]
fn diff_vaqum_archive_against_plain_file_transparently_decompresses() {
    let dir = TempDir::new().unwrap();
    let original = dir.path().join("original.txt");
    write_sample_text(&original, 50);
    let archive = dir.path().join("original.txt.vqm");
    vaqum()
        .args(["compress", original.to_str().unwrap()])
        .assert()
        .success();

    vaqum()
        .args([
            "diff",
            archive.to_str().unwrap(),
            original.to_str().unwrap(),
        ])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("identical"));
}

#[test]
fn diff_dedup_archive_against_live_directory_transparently_resolves() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src_tree");
    build_sample_tree(&src);
    let archive = dir.path().join("tree.vqm");
    vaqum()
        .args([
            "compress",
            src.to_str().unwrap(),
            "-r",
            "--dedup",
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .success();

    vaqum()
        .args(["diff", archive.to_str().unwrap(), src.to_str().unwrap()])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("identical"));
}

#[test]
fn diff_html_report_is_written_and_contains_expected_markers() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    fs::write(&a, "line1\nline2\n").unwrap();
    fs::write(&b, "line1\nCHANGED\n").unwrap();
    let report = dir.path().join("report.html");

    vaqum()
        .args([
            "diff",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "--html",
            report.to_str().unwrap(),
        ])
        .assert()
        .code(1);

    let html = fs::read_to_string(&report).unwrap();
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("CHANGED"));
    assert!(
        html.contains("sbs-diff"),
        "should render as a side-by-side table"
    );
    assert!(html.contains("class=\"add\""));
    assert!(html.contains("class=\"del\""));
}

#[test]
#[cfg(unix)]
fn diff_editor_launches_configured_command_with_diff_flag() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    fs::write(&a, "line1\nline2\n").unwrap();
    fs::write(&b, "line1\nCHANGED\n").unwrap();

    // A fake "editor" that just records how it was invoked, so this test
    // doesn't depend on VS Code (or any editor) being installed.
    let log = dir.path().join("editor-invocations.log");
    let fake_editor = dir.path().join("fake-editor.sh");
    fs::write(
        &fake_editor,
        format!("#!/bin/sh\necho \"$@\" >> \"{}\"\n", log.display()),
    )
    .unwrap();
    fs::set_permissions(&fake_editor, fs::Permissions::from_mode(0o755)).unwrap();

    vaqum()
        .args(["diff", a.to_str().unwrap(), b.to_str().unwrap(), "--editor"])
        .env("VAQUM_DIFF_EDITOR", &fake_editor)
        .assert()
        .code(1);

    let log_contents = fs::read_to_string(&log).unwrap();
    assert!(log_contents.contains("--diff"));
    assert!(log_contents.contains(a.to_str().unwrap()));
    assert!(log_contents.contains(b.to_str().unwrap()));
}

#[test]
#[cfg(unix)]
fn diff_editor_on_vaqum_archive_uses_a_scratch_copy() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let original = dir.path().join("original.txt");
    write_sample_text(&original, 20);
    let archive = dir.path().join("original.txt.vqm");
    vaqum()
        .args(["compress", original.to_str().unwrap()])
        .assert()
        .success();

    let changed = dir.path().join("changed.txt");
    let mut contents = fs::read_to_string(&original).unwrap();
    contents.push_str("one more line\n");
    fs::write(&changed, contents).unwrap();

    let log = dir.path().join("editor-invocations.log");
    let fake_editor = dir.path().join("fake-editor.sh");
    fs::write(
        &fake_editor,
        format!("#!/bin/sh\necho \"$@\" >> \"{}\"\n", log.display()),
    )
    .unwrap();
    fs::set_permissions(&fake_editor, fs::Permissions::from_mode(0o755)).unwrap();

    vaqum()
        .args([
            "diff",
            archive.to_str().unwrap(),
            changed.to_str().unwrap(),
            "--editor",
        ])
        .env("VAQUM_DIFF_EDITOR", &fake_editor)
        .assert()
        .code(1)
        .stdout(predicate::str::contains("scratch cop"));

    let log_contents = fs::read_to_string(&log).unwrap();
    assert!(log_contents.contains("--diff"));
    // The archive side must NOT be opened at its own .vqm path (that's
    // compressed bytes, not text) — it should be a materialized scratch
    // file elsewhere.
    assert!(!log_contents.contains(archive.to_str().unwrap()));
    assert!(log_contents.contains(changed.to_str().unwrap()));
}

// ---------------------------------------------------------------------
// completions
// ---------------------------------------------------------------------

#[test]
fn completions_prints_a_nonempty_script_for_each_supported_shell() {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        vaqum()
            .args(["completions", shell])
            .assert()
            .success()
            .stdout(predicate::str::contains("vaqum"))
            .stdout(predicate::str::contains("compress"));
    }
}

#[test]
fn completions_rejects_an_unsupported_shell() {
    vaqum().args(["completions", "cobol"]).assert().failure();
}

#[test]
fn completions_requires_a_shell_without_install() {
    vaqum().args(["completions"]).assert().failure();
}

#[test]
fn completions_install_writes_to_the_shell_specific_path() {
    let home = TempDir::new().unwrap();

    vaqum()
        .args(["completions", "zsh", "--install"])
        .env("HOME", home.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed zsh completions"));

    let installed = home.path().join(".zfunc/_vaqum");
    assert!(installed.exists());
    assert!(fs::read_to_string(&installed).unwrap().contains("vaqum"));
}

#[test]
fn completions_install_auto_detects_shell_from_env() {
    let home = TempDir::new().unwrap();

    vaqum()
        .args(["completions", "--install"])
        .env("HOME", home.path())
        .env("SHELL", "/bin/bash")
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed bash completions"));

    assert!(
        home.path()
            .join(".local/share/bash-completion/completions/vaqum")
            .exists()
    );
}

#[test]
fn completions_install_rejects_an_undetectable_shell() {
    let home = TempDir::new().unwrap();

    vaqum()
        .args(["completions", "--install"])
        .env("HOME", home.path())
        .env("SHELL", "/bin/tcsh")
        .assert()
        .failure()
        .stderr(predicate::str::contains("--install zsh"));
}

// ---------------------------------------------------------------------
// dedupe
// ---------------------------------------------------------------------

#[test]
fn dedupe_accepts_quiet_flag() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("photos");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("img1.txt"), "same content\n").unwrap();

    vaqum()
        .args(["dedupe", root.to_str().unwrap(), "-q"])
        .assert()
        .success();
}

#[test]
fn dedupe_reports_duplicate_groups_and_reclaimable_space() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("photos");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("img1.txt"), "same content\n").unwrap();
    fs::write(root.join("img2.txt"), "same content\n").unwrap();
    fs::write(root.join("img3.txt"), "different content\n").unwrap();

    vaqum()
        .args(["dedupe", root.to_str().unwrap(), "-v"])
        .assert()
        .success()
        .stdout(predicate::str::contains("duplicate groups:  1"))
        .stdout(predicate::str::contains("img1.txt"))
        .stdout(predicate::str::contains("img2.txt"));
}

#[test]
fn dedupe_link_hardlinks_duplicates_without_changing_content() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("photos");
    fs::create_dir_all(&root).unwrap();
    let f1 = root.join("img1.txt");
    let f2 = root.join("img2.txt");
    fs::write(&f1, "same content\n").unwrap();
    fs::write(&f2, "same content\n").unwrap();

    vaqum()
        .args(["dedupe", root.to_str().unwrap(), "--link"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hardlinked 1"));

    assert_eq!(fs::read(&f1).unwrap(), fs::read(&f2).unwrap());

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            fs::metadata(&f1).unwrap().ino(),
            fs::metadata(&f2).unwrap().ino(),
            "linked files should share an inode"
        );
    }
}

#[test]
fn dedupe_dry_run_link_does_not_modify_anything() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("photos");
    fs::create_dir_all(&root).unwrap();
    let f1 = root.join("img1.txt");
    let f2 = root.join("img2.txt");
    fs::write(&f1, "same content\n").unwrap();
    fs::write(&f2, "same content\n").unwrap();

    vaqum()
        .args(["dedupe", root.to_str().unwrap(), "--link", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("(dry run)"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_ne!(
            fs::metadata(&f1).unwrap().ino(),
            fs::metadata(&f2).unwrap().ino(),
            "dry-run must not actually link files"
        );
    }
}

// ---------------------------------------------------------------------
// search
// ---------------------------------------------------------------------

fn build_search_tree(root: &std::path::Path) {
    fs::create_dir_all(root.join("src/nested")).unwrap();
    fs::write(
        root.join("src/main.rs"),
        "fn main() {\n    // TODO: handle errors\n}\n",
    )
    .unwrap();
    fs::write(root.join("src/nested/util.rs"), "// TODO: implement this\n").unwrap();
    fs::write(root.join("README.md"), "no markers here\n").unwrap();
    fs::create_dir_all(root.join("TODO_folder")).unwrap();
    fs::write(root.join("src/blob.bin"), [0u8, 159, 146, 150, 0, 1, 2]).unwrap();
}

#[test]
fn search_default_mode_finds_both_name_and_content_matches() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("tree");
    build_search_tree(&root);

    vaqum()
        .args(["search", "TODO", root.to_str().unwrap()])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("name     "))
        .stdout(predicate::str::contains("TODO_folder"))
        .stdout(predicate::str::contains("content  "))
        // Path separator is platform-dependent (`/` vs `\`), so match on
        // the filename:line fragment only, not the full relative path.
        .stdout(predicate::str::contains("main.rs:2:"))
        .stdout(predicate::str::contains("util.rs:1:"));
}

#[test]
fn search_names_only_excludes_content_matches() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("tree");
    build_search_tree(&root);

    vaqum()
        .args(["search", "TODO", root.to_str().unwrap(), "--names-only"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("name     "))
        .stdout(predicate::str::contains("TODO_folder"))
        .stdout(predicate::str::contains("content  ").not());
}

#[test]
fn search_content_only_excludes_name_matches() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("tree");
    build_search_tree(&root);

    vaqum()
        .args(["search", "TODO", root.to_str().unwrap(), "--content-only"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("content  "))
        .stdout(predicate::str::contains("name     ").not());
}

#[test]
fn search_skips_binary_file_content() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("tree");
    build_search_tree(&root);

    // The binary file's raw bytes never match as text content, and the
    // scan must not choke or emit garbage for it.
    vaqum()
        .args(["search", "TODO", root.to_str().unwrap()])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("blob.bin").not());
}

#[test]
fn search_regex_mode() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("tree");
    build_search_tree(&root);

    vaqum()
        .args([
            "search",
            r"TODO: \w+",
            root.to_str().unwrap(),
            "-E",
            "--content-only",
        ])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("handle errors"))
        .stdout(predicate::str::contains("implement this"));
}

#[test]
fn search_case_insensitive() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("tree");
    build_search_tree(&root);

    vaqum()
        .args([
            "search",
            "todo",
            root.to_str().unwrap(),
            "-i",
            "--content-only",
        ])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("handle errors"));
}

#[test]
fn search_no_matches_exits_one() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("tree");
    build_search_tree(&root);

    vaqum()
        .args([
            "search",
            "definitely_not_present_xyz",
            root.to_str().unwrap(),
        ])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("no matches"));
}

#[test]
fn search_missing_path_exits_two() {
    vaqum()
        .args(["search", "x", "/no/such/path/at/all"])
        .assert()
        .code(2);
}

#[test]
fn search_single_file_directly() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("tree");
    build_search_tree(&root);

    vaqum()
        .args(["search", "TODO", root.join("src/main.rs").to_str().unwrap()])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("content  "))
        .stdout(predicate::str::contains("handle errors"));
}

#[test]
fn search_names_only_and_content_only_are_mutually_exclusive() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("tree");
    build_search_tree(&root);

    vaqum()
        .args([
            "search",
            "TODO",
            root.to_str().unwrap(),
            "--names-only",
            "--content-only",
        ])
        .assert()
        .failure();
}

// ---------------------------------------------------------------------
// config: show / path / init, and compress reading defaults/profiles
// ---------------------------------------------------------------------

/// A fresh, isolated `XDG_CONFIG_HOME` for a test that writes its own
/// `vaqum/config.toml`, distinct from the empty one every test gets by
/// default via `vaqum()`.
fn config_home() -> TempDir {
    TempDir::new().unwrap()
}

fn write_config(home: &TempDir, toml: &str) -> std::path::PathBuf {
    let dir = home.path().join("vaqum");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    fs::write(&path, toml).unwrap();
    path
}

#[test]
fn config_path_prints_the_resolved_location() {
    vaqum()
        .args(["config", "path"])
        .assert()
        .success()
        .stdout(predicate::str::contains("vaqum").and(predicate::str::contains("config.toml")));
}

#[test]
fn config_init_writes_a_starter_file_and_refuses_to_overwrite() {
    let home = config_home();

    vaqum()
        .args(["config", "init"])
        .env("XDG_CONFIG_HOME", home.path())
        .env("APPDATA", home.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Wrote starter config"));

    let path = home.path().join("vaqum").join("config.toml");
    assert!(path.exists(), "config init should create the file");
    assert!(!fs::read_to_string(&path).unwrap().is_empty());

    vaqum()
        .args(["config", "init"])
        .env("XDG_CONFIG_HOME", home.path())
        .env("APPDATA", home.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn config_show_without_a_file_reports_built_in_defaults() {
    vaqum()
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not found"))
        .stdout(predicate::str::contains("profiles: (none)"));
}

#[test]
fn config_show_reports_resolved_defaults_and_profiles() {
    let home = config_home();
    write_config(
        &home,
        "[defaults]\nlevel = 5\n\n[profile.backup]\nmax = true\n",
    );

    vaqum()
        .args(["config", "show"])
        .env("XDG_CONFIG_HOME", home.path())
        .env("APPDATA", home.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("level:    5"))
        .stdout(predicate::str::contains("backup:"))
        .stdout(predicate::str::contains("max:      true"));
}

#[test]
fn compress_applies_config_defaults_exclude_without_a_profile() {
    let home = config_home();
    write_config(&home, "[defaults]\nexclude = [\"*.log\"]\n");

    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("keep.txt"), "kept\n").unwrap();
    fs::write(src.join("skip.log"), "skipped\n").unwrap();

    let archive = dir.path().join("out.vqm");
    vaqum()
        .args([
            "compress",
            src.to_str().unwrap(),
            "-r",
            "-o",
            archive.to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", home.path())
        .env("APPDATA", home.path())
        .assert()
        .success();

    let out_dir = dir.path().join("out");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out_dir.to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", home.path())
        .env("APPDATA", home.path())
        .assert()
        .success();

    let restored = out_dir.join("src");
    assert!(restored.join("keep.txt").exists());
    assert!(!restored.join("skip.log").exists());
}

#[test]
fn compress_profile_flag_merges_with_defaults_and_applies_max() {
    let home = config_home();
    write_config(
        &home,
        "[defaults]\nexclude = [\"*.log\"]\n\n[profile.archive]\nmax = true\nexclude = [\"*.tmp\"]\n",
    );

    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("keep.txt"), "kept\n").unwrap();
    fs::write(src.join("skip.log"), "skipped\n").unwrap();
    fs::write(src.join("skip.tmp"), "skipped\n").unwrap();

    let archive = dir.path().join("out.vqm");
    vaqum()
        .args([
            "compress",
            src.to_str().unwrap(),
            "-r",
            "--profile",
            "archive",
            "-o",
            archive.to_str().unwrap(),
        ])
        .env("XDG_CONFIG_HOME", home.path())
        .env("APPDATA", home.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("xz (LZMA)"));

    vaqum()
        .args(["info", archive.to_str().unwrap()])
        .env("XDG_CONFIG_HOME", home.path())
        .env("APPDATA", home.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("xz (LZMA)"));
}

#[test]
fn compress_unknown_profile_fails_with_a_clear_error() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("file.txt");
    write_sample_text(&input, 50);

    vaqum()
        .args([
            "compress",
            input.to_str().unwrap(),
            "--profile",
            "does-not-exist",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "no profile named 'does-not-exist'",
        ));
}

// ---------------------------------------------------------------------
// reading foreign archives: zip, tar.gz
// ---------------------------------------------------------------------

fn build_zip_fixture(path: &std::path::Path) {
    use std::io::Write as _;
    let file = fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("a.txt", options).unwrap();
    zip.write_all(b"hello from zip\n").unwrap();
    zip.add_directory("sub/", options).unwrap();
    zip.start_file("sub/b.txt", options).unwrap();
    zip.write_all(b"nested\n").unwrap();
    zip.finish().unwrap();
}

fn build_tar_gz_fixture(path: &std::path::Path) {
    let file = fs::File::create(path).unwrap();
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    builder
        .append_data(
            &mut tar_header(b"hello from tar.gz\n".len() as u64),
            "a.txt",
            &b"hello from tar.gz\n"[..],
        )
        .unwrap();
    builder
        .append_data(
            &mut tar_header(b"nested\n".len() as u64),
            "sub/b.txt",
            &b"nested\n"[..],
        )
        .unwrap();
    builder.into_inner().unwrap().finish().unwrap();
}

fn tar_header(size: u64) -> tar::Header {
    let mut header = tar::Header::new_gnu();
    header.set_size(size);
    header.set_mode(0o644);
    header.set_cksum();
    header
}

#[test]
fn decompress_reads_a_zip_archive() {
    let dir = TempDir::new().unwrap();
    let archive = dir.path().join("archive.zip");
    build_zip_fixture(&archive);

    let out_dir = dir.path().join("out");
    vaqum()
        .args(["decompress", archive.to_str().unwrap(), "-o"])
        .arg(&out_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Extracted 2 file(s) from zip archive",
        ));

    assert_eq!(
        fs::read_to_string(out_dir.join("a.txt")).unwrap(),
        "hello from zip\n"
    );
    assert_eq!(
        fs::read_to_string(out_dir.join("sub/b.txt")).unwrap(),
        "nested\n"
    );
}

#[test]
fn decompress_reads_a_tar_gz_archive() {
    let dir = TempDir::new().unwrap();
    let archive = dir.path().join("archive.tar.gz");
    build_tar_gz_fixture(&archive);

    let out_dir = dir.path().join("out");
    vaqum()
        .args(["decompress", archive.to_str().unwrap(), "-o"])
        .arg(&out_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Extracted 2 file(s) from tar.gz archive",
        ));

    assert_eq!(
        fs::read_to_string(out_dir.join("a.txt")).unwrap(),
        "hello from tar.gz\n"
    );
    assert_eq!(
        fs::read_to_string(out_dir.join("sub/b.txt")).unwrap(),
        "nested\n"
    );
}

#[test]
fn info_on_a_zip_archive_shows_file_count_and_sizes() {
    let dir = TempDir::new().unwrap();
    let archive = dir.path().join("archive.zip");
    build_zip_fixture(&archive);

    vaqum()
        .args(["info", archive.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("type:         zip archive"))
        .stdout(predicate::str::contains("files:        2"));
}

#[test]
fn info_on_a_tar_gz_archive_shows_file_count_and_sizes() {
    let dir = TempDir::new().unwrap();
    let archive = dir.path().join("archive.tar.gz");
    build_tar_gz_fixture(&archive);

    vaqum()
        .args(["info", archive.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("type:         tar.gz archive"))
        .stdout(predicate::str::contains("files:        2"));
}

#[test]
fn decompress_rejects_a_plain_non_archive_file_with_a_clear_error() {
    let dir = TempDir::new().unwrap();
    let bogus = dir.path().join("notes.txt");
    fs::write(&bogus, b"just some text, not an archive").unwrap();

    vaqum()
        .args(["decompress", bogus.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "is not a .vqm, zip, or tar.gz archive",
        ));
}

// ---------------------------------------------------------------------
// list
// ---------------------------------------------------------------------

/// Compress `build_sample_tree` into an archive and return its path.
fn sample_archive(dir: &std::path::Path, extra: &[&str]) -> std::path::PathBuf {
    let src = dir.join("src_tree");
    if !src.exists() {
        build_sample_tree(&src);
    }
    let label = extra
        .first()
        .map(|s| s.trim_start_matches('-'))
        .unwrap_or("plain");
    let archive = dir.join(format!("sample-{label}.vqm"));
    let mut args = vec![
        "compress",
        src.to_str().unwrap(),
        "-r",
        "-o",
        archive.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    vaqum().args(&args).assert().success();
    archive
}

fn list_json(archive: &std::path::Path, extra: &[&str]) -> serde_json::Value {
    let mut args = vec!["list", "--json", archive.to_str().unwrap()];
    args.extend_from_slice(extra);
    let output = vaqum().args(&args).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn listed_paths(listing: &serde_json::Value) -> Vec<String> {
    listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn list_directory_archive_shows_paths_under_the_archive_name() {
    let dir = TempDir::new().unwrap();
    let archive = sample_archive(dir.path(), &[]);
    vaqum()
        .args(["list", archive.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("src_tree/a.txt"))
        .stdout(predicate::str::contains("src_tree/nested/c.txt"))
        .stdout(predicate::str::contains("src_tree/nested/"))
        .stdout(predicate::str::contains("3 files, 1 directory"));
}

#[test]
fn list_ls_alias_works() {
    let dir = TempDir::new().unwrap();
    let archive = sample_archive(dir.path(), &[]);
    vaqum()
        .args(["ls", archive.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("src_tree/a.txt"));
}

#[test]
fn list_json_reports_entries_kinds_and_sizes() {
    let dir = TempDir::new().unwrap();
    let archive = sample_archive(dir.path(), &[]);
    let listing = list_json(&archive, &[]);
    assert_eq!(listing["format"], "vaqum");
    assert_eq!(listing["kind"], "directory archive");
    assert_eq!(listing["files"], 3);
    assert_eq!(listing["dirs"], 1);
    let a_size = fs::metadata(dir.path().join("src_tree/a.txt"))
        .unwrap()
        .len();
    let a = listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"] == "src_tree/a.txt")
        .unwrap();
    assert_eq!(a["kind"], "file");
    assert_eq!(a["size"], a_size);
}

#[test]
fn list_dedup_archive_shows_real_paths_not_internal_objects() {
    let dir = TempDir::new().unwrap();
    let archive = sample_archive(dir.path(), &["--dedup"]);
    let listing = list_json(&archive, &[]);
    let mut paths = listed_paths(&listing);
    paths.sort();
    assert_eq!(
        paths,
        [
            "src_tree/a.txt",
            "src_tree/b.txt",
            "src_tree/nested",
            "src_tree/nested/c.txt"
        ]
    );
    // b.txt is a duplicate of a.txt but still reports its real size.
    let sizes: Vec<u64> = listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["path"] == "src_tree/a.txt" || e["path"] == "src_tree/b.txt")
        .map(|e| e["size"].as_u64().unwrap())
        .collect();
    assert_eq!(sizes.len(), 2);
    assert_eq!(sizes[0], sizes[1]);
    assert!(sizes[0] > 0);
}

#[test]
fn list_encrypted_archive_needs_the_right_key() {
    let dir = TempDir::new().unwrap();
    let key = dir.path().join("key.bin");
    fs::write(&key, b"correct horse battery staple").unwrap();
    let archive = sample_archive(dir.path(), &["--key-file", key.to_str().unwrap()]);

    let listing = list_json(&archive, &["--key-file", key.to_str().unwrap()]);
    assert_eq!(listing["encrypted"], true);
    assert!(listed_paths(&listing).contains(&"src_tree/a.txt".to_string()));

    let wrong = dir.path().join("wrong.bin");
    fs::write(&wrong, b"nope").unwrap();
    vaqum()
        .args([
            "list",
            archive.to_str().unwrap(),
            "--key-file",
            wrong.to_str().unwrap(),
        ])
        .assert()
        .failure();
}

#[test]
fn list_single_file_archive_shows_the_one_file() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("notes.txt");
    write_sample_text(&input, 100);
    vaqum()
        .args(["compress", input.to_str().unwrap()])
        .assert()
        .success();
    let listing = list_json(&dir.path().join("notes.txt.vqm"), &[]);
    assert_eq!(listed_paths(&listing), ["notes.txt"]);
    assert_eq!(
        listing["entries"][0]["size"],
        fs::metadata(&input).unwrap().len()
    );
}

#[test]
fn list_reads_tar_gz_archives() {
    let dir = TempDir::new().unwrap();
    let tgz = dir.path().join("t.tar.gz");
    {
        let enc = flate2::write::GzEncoder::new(
            fs::File::create(&tgz).unwrap(),
            flate2::Compression::default(),
        );
        let mut builder = tar::Builder::new(enc);
        let data = b"hello";
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "dir/hello.txt", &data[..])
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();
    }
    let listing = list_json(&tgz, &[]);
    assert_eq!(listing["format"], "tar.gz");
    assert_eq!(listed_paths(&listing), ["dir/hello.txt"]);
}

#[test]
fn list_rejects_a_non_archive() {
    let dir = TempDir::new().unwrap();
    let f = dir.path().join("plain.txt");
    fs::write(&f, "just text").unwrap();
    vaqum()
        .args(["list", f.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not a .vqm"));
}

// ---------------------------------------------------------------------
// decompress --only
// ---------------------------------------------------------------------

#[test]
fn only_extracts_just_the_named_file() {
    let dir = TempDir::new().unwrap();
    let archive = sample_archive(dir.path(), &[]);
    let out = dir.path().join("out");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--only",
            "src_tree/nested/c.txt",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Extracted 1 file"));
    assert_eq!(walk_relative(&out), ["src_tree/nested/c.txt"]);
    assert_eq!(
        fs::read(out.join("src_tree/nested/c.txt")).unwrap(),
        fs::read(dir.path().join("src_tree/nested/c.txt")).unwrap()
    );
}

#[test]
fn only_accepts_paths_without_the_archive_name_prefix() {
    let dir = TempDir::new().unwrap();
    let archive = sample_archive(dir.path(), &[]);
    let out = dir.path().join("out");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--only",
            "./a.txt",
        ])
        .assert()
        .success();
    assert_eq!(walk_relative(&out), ["src_tree/a.txt"]);
}

#[test]
fn only_with_a_directory_extracts_everything_inside_it() {
    let dir = TempDir::new().unwrap();
    let archive = sample_archive(dir.path(), &[]);
    let out = dir.path().join("out");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--only",
            "src_tree/nested/",
            "--only",
            "src_tree/b.txt",
        ])
        .assert()
        .success();
    let mut files = walk_relative(&out);
    files.sort();
    assert_eq!(files, ["src_tree/b.txt", "src_tree/nested/c.txt"]);
}

#[test]
fn only_fails_clearly_when_a_path_is_not_in_the_archive() {
    let dir = TempDir::new().unwrap();
    let archive = sample_archive(dir.path(), &[]);
    let out = dir.path().join("out");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--only",
            "src_tree/missing.txt",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "nothing in the archive matches: src_tree/missing.txt",
        ))
        .stderr(predicate::str::contains("vaqum list"));
}

#[test]
fn only_with_verify_still_checks_the_whole_archive() {
    let dir = TempDir::new().unwrap();
    let archive = sample_archive(dir.path(), &[]);
    let out = dir.path().join("out");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--only",
            "src_tree/a.txt",
            "--verify",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("verified checksum"));
    assert_eq!(walk_relative(&out), ["src_tree/a.txt"]);
}

#[test]
fn only_works_on_dedup_archives_including_duplicates() {
    let dir = TempDir::new().unwrap();
    let archive = sample_archive(dir.path(), &["--dedup"]);
    let out = dir.path().join("out");
    // b.txt's content is stored once, under a.txt's object.
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--only",
            "src_tree/b.txt",
            "--verify",
        ])
        .assert()
        .success();
    assert_eq!(walk_relative(&out), ["src_tree/b.txt"]);
    assert_eq!(
        fs::read(out.join("src_tree/b.txt")).unwrap(),
        fs::read(dir.path().join("src_tree/b.txt")).unwrap()
    );
}

#[test]
fn only_works_on_multi_path_bundles() {
    let dir = TempDir::new().unwrap();
    let one = dir.path().join("one.txt");
    let two = dir.path().join("two.txt");
    write_sample_text(&one, 50);
    write_sample_text(&two, 60);
    let archive = dir.path().join("bundle.vqm");
    vaqum()
        .args([
            "compress",
            one.to_str().unwrap(),
            two.to_str().unwrap(),
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .success();
    let out = dir.path().join("out");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--only",
            "two.txt",
        ])
        .assert()
        .success();
    assert_eq!(walk_relative(&out), ["two.txt"]);
}

// ---------------------------------------------------------------------
// stdin / stdout
// ---------------------------------------------------------------------

#[test]
fn stdin_to_stdout_round_trips_and_keeps_status_off_stdout() {
    let data: Vec<u8> = (0..200_000u32)
        .flat_map(|i| (i % 251).to_le_bytes())
        .collect();

    let compressed = vaqum()
        .args(["compress", "-"])
        .write_stdin(data.clone())
        .output()
        .unwrap();
    assert!(
        compressed.status.success(),
        "{}",
        String::from_utf8_lossy(&compressed.stderr)
    );
    assert_eq!(
        &compressed.stdout[..4],
        b"VAQM",
        "stdout must carry only the archive"
    );
    assert!(String::from_utf8_lossy(&compressed.stderr).contains("Compressed 'stdin' -> 'stdout'"));

    let restored = vaqum()
        .args(["decompress", "-", "-o", "-", "--verify"])
        .write_stdin(compressed.stdout)
        .output()
        .unwrap();
    assert!(
        restored.status.success(),
        "{}",
        String::from_utf8_lossy(&restored.stderr)
    );
    assert_eq!(restored.stdout, data);
    assert!(String::from_utf8_lossy(&restored.stderr).contains("verified checksum"));
}

#[test]
fn stdin_compressed_to_a_file_is_named_after_the_output() {
    let dir = TempDir::new().unwrap();
    let archive = dir.path().join("report.csv.vqm");
    vaqum()
        .args(["compress", "-", "-o", archive.to_str().unwrap()])
        .write_stdin("a,b\n1,2\n")
        .assert()
        .success();
    let out = dir.path().join("out");
    fs::create_dir_all(&out).unwrap();
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(out.join("report.csv")).unwrap(),
        "a,b\n1,2\n"
    );
}

#[test]
fn stdin_cannot_be_mixed_with_other_inputs() {
    let dir = TempDir::new().unwrap();
    let f = dir.path().join("f.txt");
    fs::write(&f, "x").unwrap();
    vaqum()
        .args([
            "compress",
            "-",
            f.to_str().unwrap(),
            "-o",
            dir.path().join("o.vqm").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("can't be combined"));
}

#[test]
fn encrypted_archive_round_trips_through_pipes_with_a_key_file() {
    let dir = TempDir::new().unwrap();
    let key = dir.path().join("key.bin");
    fs::write(&key, b"pipe secret").unwrap();
    let compressed = vaqum()
        .args(["compress", "-", "--key-file", key.to_str().unwrap()])
        .write_stdin("top secret payload")
        .output()
        .unwrap();
    assert!(compressed.status.success());
    let restored = vaqum()
        .args([
            "decompress",
            "-",
            "-o",
            "-",
            "--key-file",
            key.to_str().unwrap(),
        ])
        .write_stdin(compressed.stdout)
        .output()
        .unwrap();
    assert!(
        restored.status.success(),
        "{}",
        String::from_utf8_lossy(&restored.stderr)
    );
    assert_eq!(restored.stdout, b"top secret payload");
}

#[test]
fn directory_archive_to_stdout_is_a_plain_tar_stream() {
    let dir = TempDir::new().unwrap();
    let archive = sample_archive(dir.path(), &[]);
    let output = vaqum()
        .args(["decompress", archive.to_str().unwrap(), "-o", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut tar = tar::Archive::new(output.stdout.as_slice());
    let mut names: Vec<String> = tar
        .entries()
        .unwrap()
        .map(|e| {
            e.unwrap()
                .path()
                .unwrap()
                .to_string_lossy()
                .trim_end_matches('/')
                .to_string()
        })
        .collect();
    names.sort();
    assert_eq!(names, ["a.txt", "b.txt", "nested", "nested/c.txt"]);
}

#[test]
fn dedup_archive_to_stdout_is_refused() {
    let dir = TempDir::new().unwrap();
    let archive = sample_archive(dir.path(), &["--dedup"]);
    vaqum()
        .args(["decompress", archive.to_str().unwrap(), "-o", "-"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("isn't supported for --dedup"));
}

#[test]
fn list_reads_an_archive_from_stdin() {
    let dir = TempDir::new().unwrap();
    let archive = sample_archive(dir.path(), &[]);
    vaqum()
        .args(["list", "-"])
        .write_stdin(fs::read(&archive).unwrap())
        .assert()
        .success()
        .stdout(predicate::str::contains("src_tree/nested/c.txt"));
}

// ---------------------------------------------------------------------
// extension
// ---------------------------------------------------------------------

#[test]
fn archives_with_the_old_vaqum_extension_still_work() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("old.txt");
    write_sample_text(&input, 100);
    let archive = dir.path().join("old.txt.vaqum");
    vaqum()
        .args([
            "compress",
            input.to_str().unwrap(),
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .success();
    let out = dir.path().join("out");
    fs::create_dir_all(&out).unwrap();
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--verify",
        ])
        .assert()
        .success();
    assert_eq!(
        fs::read(&input).unwrap(),
        fs::read(out.join("old.txt")).unwrap()
    );
    vaqum()
        .args(["list", archive.to_str().unwrap()])
        .assert()
        .success();
}

// ---------------------------------------------------------------------
// shred warning
// ---------------------------------------------------------------------

#[test]
fn shred_warns_about_ssds_and_copy_on_write_before_confirming() {
    let dir = TempDir::new().unwrap();
    let f = dir.path().join("secret.txt");
    fs::write(&f, "secret").unwrap();
    vaqum()
        .args(["shred", f.to_str().unwrap(), "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Copy-on-write filesystems"))
        .stdout(predicate::str::contains("full-disk encryption"));
    assert!(f.exists());
}

// ---------------------------------------------------------------------
// security: hostile archives
// ---------------------------------------------------------------------

/// A raw `.vqm` header, so tests can build archives vaqum itself would
/// never write.
fn raw_header(entry_type: u8, flags: u8, original_size: u64, name: &str) -> Vec<u8> {
    let mut h = Vec::new();
    h.extend_from_slice(b"VAQM");
    h.extend_from_slice(&[2, entry_type, 0, flags]);
    h.extend_from_slice(&original_size.to_le_bytes());
    h.extend_from_slice(&[0u8; 32 + 16 + 7]);
    h.extend_from_slice(&(name.len() as u16).to_le_bytes());
    h.extend_from_slice(name.as_bytes());
    h
}

fn write_raw_archive(
    path: &std::path::Path,
    entry_type: u8,
    flags: u8,
    size: u64,
    name: &str,
    payload: &[u8],
) {
    let mut bytes = raw_header(entry_type, flags, size, name);
    bytes.extend(zstd::encode_all(payload, 3).unwrap());
    fs::write(path, bytes).unwrap();
}

/// A dedup tar whose manifest maps `path` to an object with `hash`.
fn dedup_tar_with(path: &str, hash: &str, data: &[u8]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    let mut h = tar::Header::new_gnu();
    h.set_size(data.len() as u64);
    h.set_mode(0o644);
    h.set_cksum();
    builder
        .append_data(&mut h, format!(".vaqum-objects/{}", "a".repeat(64)), data)
        .unwrap();
    let manifest =
        serde_json::json!({ "dirs": [], "files": [{ "path": path, "hash": hash, "mode": null }] });
    let m = serde_json::to_vec(&manifest).unwrap();
    let mut h = tar::Header::new_gnu();
    h.set_size(m.len() as u64);
    h.set_mode(0o644);
    h.set_cksum();
    builder
        .append_data(&mut h, ".vaqum-manifest.json", m.as_slice())
        .unwrap();
    builder.into_inner().unwrap()
}

#[test]
fn archive_with_a_path_traversal_name_is_refused() {
    let dir = TempDir::new().unwrap();
    let archive = dir.path().join("evil.vqm");
    write_raw_archive(&archive, 0, 0, 6, "../escaped.txt", b"pwned\n");
    let out = dir.path().join("out");
    fs::create_dir_all(&out).unwrap();
    for cmd in [
        vec![
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ],
        vec!["list", archive.to_str().unwrap()],
    ] {
        vaqum()
            .args(&cmd)
            .assert()
            .failure()
            .stderr(predicate::str::contains("isn't a plain file name"));
    }
    assert!(
        !dir.path().join("escaped.txt").exists(),
        "file escaped the output directory"
    );
}

#[test]
fn archive_with_an_absolute_name_is_refused() {
    let dir = TempDir::new().unwrap();
    let archive = dir.path().join("evil.vqm");
    let target = dir.path().join("absolute-target.txt");
    write_raw_archive(&archive, 0, 0, 6, target.to_str().unwrap(), b"pwned\n");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            dir.path().join("out").to_str().unwrap(),
        ])
        .assert()
        .failure();
    assert!(!target.exists());
}

#[test]
fn dedup_manifest_path_traversal_is_refused() {
    let dir = TempDir::new().unwrap();
    let tar = dedup_tar_with("../escaped.txt", &"a".repeat(64), b"pwned\n");
    let archive = dir.path().join("evil.vqm");
    write_raw_archive(&archive, 1, 1, tar.len() as u64, "victim", &tar);
    let out = dir.path().join("out");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "points outside the output directory",
        ));
    assert!(!out.join("escaped.txt").exists());
    assert!(!dir.path().join("escaped.txt").exists());
    // --only goes through its own code path: must be refused too.
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--only",
            "../escaped.txt",
        ])
        .assert()
        .failure();
    assert!(!out.join("escaped.txt").exists());
}

#[test]
fn dedup_manifest_with_a_bogus_object_id_is_refused() {
    let dir = TempDir::new().unwrap();
    // The object id is used as a file name: "../../x" must not be followed.
    let tar = dedup_tar_with("ok.txt", "../../../../etc/hosts", b"data");
    let archive = dir.path().join("evil.vqm");
    write_raw_archive(&archive, 1, 1, tar.len() as u64, "victim", &tar);
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            dir.path().join("out").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid object id"));
}

#[test]
fn archive_that_lies_about_its_size_is_stopped_and_leaves_nothing_behind() {
    let dir = TempDir::new().unwrap();
    let archive = dir.path().join("bomb.vqm");
    // Claims 64 bytes, really 20 MB: would slip past the ratio and disk checks.
    write_raw_archive(&archive, 0, 0, 64, "bomb.bin", &vec![0u8; 20 * 1024 * 1024]);
    let out = dir.path().join("out");
    fs::create_dir_all(&out).unwrap();
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("possible decompression bomb"));
    assert_eq!(
        fs::read_dir(&out).unwrap().count(),
        0,
        "no partial or temp files may remain"
    );
    vaqum()
        .args(["decompress", archive.to_str().unwrap(), "-o", "-"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("possible decompression bomb"));
}

#[test]
fn zip_with_a_path_traversal_entry_cannot_escape() {
    let dir = TempDir::new().unwrap();
    let zip_path = dir.path().join("evil.zip");
    {
        let mut zip = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
        let opts: zip::write::SimpleFileOptions = Default::default();
        zip.start_file("../escaped.txt", opts).unwrap();
        zip.write_all(b"pwned\n").unwrap();
        zip.finish().unwrap();
    }
    let out = dir.path().join("out");
    let _ = vaqum()
        .args([
            "decompress",
            zip_path.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !dir.path().join("escaped.txt").exists(),
        "zip entry escaped the output directory"
    );
}

#[test]
fn tar_gz_with_a_path_traversal_entry_cannot_escape() {
    let dir = TempDir::new().unwrap();
    let tgz = dir.path().join("evil.tar.gz");
    {
        // Write the raw header by hand: the tar crate's builder refuses `..`.
        let data = b"pwned\n";
        let mut header = tar::Header::new_old();
        header.as_old_mut().name[..14].copy_from_slice(b"../escaped.txt");
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        let mut raw = header.as_bytes().to_vec();
        raw.extend_from_slice(data);
        raw.resize(512 * 2, 0);
        raw.extend_from_slice(&[0u8; 1024]);
        let mut enc = flate2::write::GzEncoder::new(
            fs::File::create(&tgz).unwrap(),
            flate2::Compression::default(),
        );
        enc.write_all(&raw).unwrap();
        enc.finish().unwrap();
    }
    let out = dir.path().join("out");
    let _ = vaqum()
        .args([
            "decompress",
            tgz.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !dir.path().join("escaped.txt").exists(),
        "tar entry escaped the output directory"
    );
}

#[cfg(unix)]
#[test]
fn shred_refuses_a_symlink_instead_of_destroying_its_target() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("important.txt");
    fs::write(&target, "do not destroy").unwrap();
    let link = dir.path().join("link.txt");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    vaqum()
        .args(["shred", link.to_str().unwrap(), "-y"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("is a symlink"));
    assert_eq!(fs::read_to_string(&target).unwrap(), "do not destroy");
}

#[cfg(unix)]
#[test]
fn compress_replaces_a_planted_symlink_instead_of_writing_through_it() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in.txt");
    write_sample_text(&input, 50);
    let victim = dir.path().join("victim.txt");
    fs::write(&victim, "keep me").unwrap();
    // An attacker pre-plants the output path as a link to another file.
    let out = dir.path().join("in.txt.vqm");
    std::os::unix::fs::symlink(&victim, &out).unwrap();
    vaqum()
        .args(["compress", input.to_str().unwrap()])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(&victim).unwrap(),
        "keep me",
        "write went through the symlink"
    );
    assert!(!fs::symlink_metadata(&out).unwrap().file_type().is_symlink());
}

#[test]
fn compress_leaves_no_temp_files_behind() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in.txt");
    write_sample_text(&input, 50);
    vaqum()
        .args(["compress", input.to_str().unwrap()])
        .assert()
        .success();
    let mut names: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["in.txt", "in.txt.vqm"]);
}

#[test]
fn archives_encrypted_by_1_2_5_still_decrypt() {
    let dir = TempDir::new().unwrap();
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    vaqum()
        .args([
            "decompress",
            fixtures
                .join("legacy-1.2.5-encrypted.vaqum")
                .to_str()
                .unwrap(),
            "-o",
            dir.path().to_str().unwrap(),
            "--key-file",
            fixtures.join("legacy.key").to_str().unwrap(),
            "--verify",
        ])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(dir.path().join("legacy.txt")).unwrap(),
        "legacy secret\n"
    );
}

#[test]
fn tampered_encrypted_archive_is_rejected() {
    let dir = TempDir::new().unwrap();
    let key = dir.path().join("k");
    fs::write(&key, "key").unwrap();
    let input = dir.path().join("s.txt");
    write_sample_text(&input, 200);
    vaqum()
        .args([
            "compress",
            input.to_str().unwrap(),
            "--key-file",
            key.to_str().unwrap(),
        ])
        .assert()
        .success();
    let archive = dir.path().join("s.txt.vqm");
    let mut bytes = fs::read(&archive).unwrap();
    let last = bytes.len() - 20;
    bytes[last] ^= 0x01; // flip one bit in the encrypted payload
    fs::write(&archive, bytes).unwrap();
    let out = dir.path().join("out");
    fs::create_dir_all(&out).unwrap();
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--key-file",
            key.to_str().unwrap(),
        ])
        .assert()
        .failure();
    assert!(
        !out.join("s.txt").exists(),
        "tampered data must not be written out"
    );
}

// ---------------------------------------------------------------------
// --only wildcards
// ---------------------------------------------------------------------

/// Archive of a small tree with mixed file types, returning (archive, out dir).
fn wildcard_archive(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let src = dir.join("proj");
    fs::create_dir_all(src.join("src/nested")).unwrap();
    fs::create_dir_all(src.join("docs/guide")).unwrap();
    for (p, c) in [
        ("src/main.rs", "fn main() {}"),
        ("src/nested/util.rs", "pub fn u() {}"),
        ("src/notes.txt", "notes"),
        ("docs/intro.md", "# intro"),
        ("docs/guide/setup.md", "# setup"),
        ("docs/logo.png", "png"),
        ("README.md", "# readme"),
    ] {
        fs::write(src.join(p), c).unwrap();
    }
    let archive = dir.join("proj.vqm");
    vaqum()
        .args([
            "compress",
            src.to_str().unwrap(),
            "-r",
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .success();
    (archive, dir.join("out"))
}

fn extracted(out: &std::path::Path) -> Vec<String> {
    let mut files = walk_relative(out);
    files.sort();
    files
}

#[test]
fn only_wildcard_without_a_slash_matches_names_at_any_depth() {
    let dir = TempDir::new().unwrap();
    let (archive, out) = wildcard_archive(dir.path());
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--only",
            "*.rs",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Extracted 2 files"));
    assert_eq!(
        extracted(&out),
        ["proj/src/main.rs", "proj/src/nested/util.rs"]
    );
}

#[test]
fn only_wildcard_with_a_slash_matches_the_path_and_double_star_spans_folders() {
    let dir = TempDir::new().unwrap();
    let (archive, out) = wildcard_archive(dir.path());
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--only",
            "docs/**/*.md",
        ])
        .assert()
        .success();
    assert_eq!(
        extracted(&out),
        ["proj/docs/guide/setup.md", "proj/docs/intro.md"]
    );

    // A single `*` stays within one folder.
    let out2 = dir.path().join("out2");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out2.to_str().unwrap(),
            "--only",
            "docs/*.md",
        ])
        .assert()
        .success();
    assert_eq!(extracted(&out2), ["proj/docs/intro.md"]);
}

#[test]
fn only_wildcards_and_paths_combine_and_braces_work() {
    let dir = TempDir::new().unwrap();
    let (archive, out) = wildcard_archive(dir.path());
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--only",
            "*.{png,txt}",
            "--only",
            "README.md",
        ])
        .assert()
        .success();
    assert_eq!(
        extracted(&out),
        ["proj/README.md", "proj/docs/logo.png", "proj/src/notes.txt"]
    );
}

#[test]
fn only_wildcard_matching_a_folder_brings_its_contents() {
    let dir = TempDir::new().unwrap();
    let (archive, out) = wildcard_archive(dir.path());
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--only",
            "gui*",
        ])
        .assert()
        .success();
    assert_eq!(extracted(&out), ["proj/docs/guide/setup.md"]);
}

#[test]
fn only_wildcard_that_matches_nothing_fails_and_bad_patterns_are_reported() {
    let dir = TempDir::new().unwrap();
    let (archive, out) = wildcard_archive(dir.path());
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--only",
            "*.zip",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "nothing in the archive matches: *.zip",
        ));
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--only",
            "[unclosed",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid --only pattern"));
}

// ---------------------------------------------------------------------
// risky-content warnings
// ---------------------------------------------------------------------

/// A tree full of tricks, compressed; returns the archive.
fn risky_archive(dir: &std::path::Path) -> std::path::PathBuf {
    let src = dir.join("parcel");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("invoice.pdf"), b"MZ\x90\x00 fake windows program").unwrap();
    fs::write(src.join("photo.jpg.exe"), b"not really").unwrap();
    fs::write(src.join("report\u{202E}fdp.exe"), b"x").unwrap();
    fs::write(src.join("readme.vbs"), b"x").unwrap();
    fs::write(src.join("inner.dat"), b"PK\x03\x04 nested zip").unwrap();
    fs::write(src.join("plain.txt"), b"perfectly normal").unwrap();
    let archive = dir.join("parcel.vqm");
    vaqum()
        .args([
            "compress",
            src.to_str().unwrap(),
            "-r",
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .success();
    archive
}

#[test]
fn list_flags_disguised_and_risky_files() {
    let dir = TempDir::new().unwrap();
    let archive = risky_archive(dir.path());
    let output = vaqum()
        .args(["list", archive.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "parcel/invoice.pdf — is named like a .pdf but is actually a Windows program"
        ),
        "{stderr}"
    );
    assert!(
        stderr.contains("photo.jpg.exe — looks like a .jpg but is really a .exe"),
        "{stderr}"
    );
    assert!(stderr.contains("invisible characters"), "{stderr}");
    assert!(
        stderr.contains("readme.vbs — runs code when opened (.vbs)"),
        "{stderr}"
    );
    assert!(
        stderr.contains("inner.dat — is another archive inside this one"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("plain.txt"),
        "harmless files must not be flagged: {stderr}"
    );

    let listing = list_json(&archive, &[]);
    let plain = listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"] == "parcel/plain.txt")
        .unwrap();
    assert!(plain.get("warnings").is_none());
    let invoice = listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"] == "parcel/invoice.pdf")
        .unwrap();
    assert!(
        invoice["warnings"][0]
            .as_str()
            .unwrap()
            .contains("Windows program")
    );
}

#[test]
fn decompress_shows_the_same_warnings_after_extracting() {
    let dir = TempDir::new().unwrap();
    let archive = risky_archive(dir.path());
    let out = dir.path().join("out");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("worth a closer look"))
        .stderr(predicate::str::contains(
            "invoice.pdf — is named like a .pdf but is actually a Windows program",
        ));
    // --only reports just what it extracted.
    let out2 = dir.path().join("out2");
    let output = vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out2.to_str().unwrap(),
            "--only",
            "plain.txt",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("closer look"));
}

#[test]
fn clean_archives_produce_no_warnings() {
    let dir = TempDir::new().unwrap();
    let archive = sample_archive(dir.path(), &[]);
    let out = dir.path().join("out");
    for args in [
        vec!["list", archive.to_str().unwrap()],
        vec![
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ],
    ] {
        let output = vaqum().args(&args).output().unwrap();
        assert!(output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("closer look"));
    }
}

#[test]
fn tar_gz_link_pointing_outside_is_flagged() {
    let dir = TempDir::new().unwrap();
    let tgz = dir.path().join("links.tar.gz");
    {
        let enc = flate2::write::GzEncoder::new(
            fs::File::create(&tgz).unwrap(),
            flate2::Compression::default(),
        );
        let mut builder = tar::Builder::new(enc);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        builder
            .append_link(&mut header, "keys", "/home/user/.ssh")
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();
    }
    vaqum()
        .args(["list", tgz.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("keys -> /home/user/.ssh"))
        .stderr(predicate::str::contains(
            "keys — is a link pointing outside the archive (to /home/user/.ssh)",
        ));
}

// ---------------------------------------------------------------------
// download mark ("downloaded from the internet")
// ---------------------------------------------------------------------

#[cfg(target_os = "macos")]
#[test]
fn quarantine_mark_is_carried_to_extracted_files() {
    let dir = TempDir::new().unwrap();
    let archive = sample_archive(dir.path(), &[]);
    let mark = b"0083;6740a3c1;Safari;F0E1D2C3-0000-0000-0000-000000000000";
    xattr::set(&archive, "com.apple.quarantine", mark).unwrap();
    let out = dir.path().join("out");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .assert()
        .success();
    for rel in ["src_tree/a.txt", "src_tree/nested/c.txt", "src_tree/nested"] {
        assert_eq!(
            xattr::get(out.join(rel), "com.apple.quarantine")
                .unwrap()
                .as_deref(),
            Some(&mark[..]),
            "{rel} lost the download mark"
        );
    }
    // Single-file archives too.
    let input = dir.path().join("one.txt");
    fs::write(&input, "x").unwrap();
    vaqum()
        .args(["compress", input.to_str().unwrap()])
        .assert()
        .success();
    let one = dir.path().join("one.txt.vqm");
    xattr::set(&one, "com.apple.quarantine", mark).unwrap();
    let out1 = dir.path().join("out1");
    fs::create_dir_all(&out1).unwrap();
    vaqum()
        .args([
            "decompress",
            one.to_str().unwrap(),
            "-o",
            out1.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert!(
        xattr::get(out1.join("one.txt"), "com.apple.quarantine")
            .unwrap()
            .is_some()
    );
}

#[cfg(target_os = "macos")]
#[test]
fn archives_without_a_mark_extract_unmarked() {
    let dir = TempDir::new().unwrap();
    let archive = sample_archive(dir.path(), &[]);
    let out = dir.path().join("out");
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert!(
        xattr::get(out.join("src_tree/a.txt"), "com.apple.quarantine")
            .unwrap()
            .is_none()
    );
}

// ---------------------------------------------------------------------
// leak check
// ---------------------------------------------------------------------

/// A tree holding things that look like secrets. Token strings are built
/// at runtime so they never appear verbatim in the repository.
fn leaky_tree(root: &std::path::Path) {
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(root.join("README.md"), "# app").unwrap();
    fs::write(root.join(".env"), "DATABASE_URL=postgres://x\n").unwrap();
    fs::write(root.join(".env.example"), "DATABASE_URL=\n").unwrap();
    fs::write(root.join("config/id_ed25519"), "key material").unwrap();
    let aws = format!("AKIA{}", "Z".repeat(16));
    fs::write(root.join("config/settings.py"), format!("AWS = '{aws}'\n")).unwrap();
    let pem = format!("-----BEGIN {} KEY-----\nabc\n", "PRIVATE");
    fs::write(root.join("config/server.txt"), pem).unwrap();
}

#[test]
fn compress_warns_about_secrets_without_printing_them() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("app");
    leaky_tree(&src);
    let output = vaqum()
        .args([
            "compress",
            src.to_str().unwrap(),
            "-r",
            "-o",
            dir.path().join("a.vqm").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Possible secrets in 4 files"), "{stderr}");
    assert!(
        stderr.contains("app/.env — looks like an environment file"),
        "{stderr}"
    );
    assert!(
        stderr.contains("app/config/id_ed25519 — looks like an SSH private key"),
        "{stderr}"
    );
    assert!(
        stderr.contains("app/config/settings.py — contains what looks like an AWS access key"),
        "{stderr}"
    );
    assert!(
        stderr.contains("app/config/server.txt — contains what looks like a private key"),
        "{stderr}"
    );
    assert!(stderr.contains("Not encrypted"), "{stderr}");
    assert!(
        !stderr.contains(".env.example"),
        "templates must not be flagged: {stderr}"
    );
    assert!(
        !stderr.contains("ZZZZ"),
        "the secret value itself must never be printed: {stderr}"
    );
}

#[test]
fn leak_check_respects_exclude_and_can_be_skipped() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("app");
    leaky_tree(&src);
    let out = dir.path().join("a.vqm");
    let excluded = vaqum()
        .args([
            "compress",
            src.to_str().unwrap(),
            "-r",
            "-o",
            out.to_str().unwrap(),
            "--exclude",
            ".env",
            "--exclude",
            "config/",
        ])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&excluded.stderr).contains("Possible secrets"));
    let skipped = vaqum()
        .args([
            "compress",
            src.to_str().unwrap(),
            "-r",
            "-o",
            out.to_str().unwrap(),
            "--no-secret-check",
        ])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&skipped.stderr).contains("Possible secrets"));
}

#[test]
fn leak_check_notes_when_the_archive_is_encrypted() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("app");
    leaky_tree(&src);
    let key = dir.path().join("k");
    fs::write(&key, "k").unwrap();
    vaqum()
        .args([
            "compress",
            src.to_str().unwrap(),
            "-r",
            "--key-file",
            key.to_str().unwrap(),
            "-o",
            dir.path().join("a.vqm").to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("The archive is encrypted"));
}

// ---------------------------------------------------------------------
// personal keys: encrypt for recipients, sign, verify
// ---------------------------------------------------------------------

struct Person {
    identity: std::path::PathBuf,
    public: String,
}

fn person(dir: &std::path::Path, name: &str) -> Person {
    let identity = dir.join(format!("{name}.key"));
    vaqum()
        .args(["keygen", "-o", identity.to_str().unwrap()])
        .assert()
        .success();
    let out = vaqum()
        .args(["pubkey", "-i", identity.to_str().unwrap()])
        .output()
        .unwrap();
    let public = String::from_utf8(out.stdout).unwrap().trim().to_string();
    assert!(public.starts_with("vqm1"), "{public}");
    Person { identity, public }
}

fn sample_input(dir: &std::path::Path) -> std::path::PathBuf {
    let input = dir.join("plans.txt");
    write_sample_text(&input, 300);
    input
}

#[test]
fn keygen_creates_a_private_identity_and_refuses_to_overwrite() {
    let dir = TempDir::new().unwrap();
    let alice = person(dir.path(), "alice");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&alice.identity).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "identity must be readable only by its owner"
        );
    }
    let before = fs::read(&alice.identity).unwrap();
    vaqum()
        .args(["keygen", "-o", alice.identity.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("refusing to overwrite"));
    assert_eq!(fs::read(&alice.identity).unwrap(), before);
}

#[test]
fn keygen_defaults_to_the_config_dir_and_is_used_automatically() {
    let dir = TempDir::new().unwrap();
    let cfg = dir.path().join("cfg");
    let with_cfg = || {
        let mut c = vaqum();
        c.env("XDG_CONFIG_HOME", &cfg).env("APPDATA", &cfg);
        c
    };
    with_cfg().arg("keygen").assert().success();
    let me = String::from_utf8(with_cfg().arg("pubkey").output().unwrap().stdout).unwrap();
    let input = sample_input(dir.path());
    let archive = dir.path().join("p.vqm");
    with_cfg()
        .args([
            "compress",
            input.to_str().unwrap(),
            "--to",
            me.trim(),
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .success();
    let out = dir.path().join("out");
    fs::create_dir_all(&out).unwrap();
    with_cfg()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--verify",
        ])
        .assert()
        .success();
    assert_eq!(
        fs::read(&input).unwrap(),
        fs::read(out.join("plans.txt")).unwrap()
    );
}

#[test]
fn archive_encrypted_for_several_people_opens_for_each_and_nobody_else() {
    let dir = TempDir::new().unwrap();
    let (alice, bob, eve) = (
        person(dir.path(), "alice"),
        person(dir.path(), "bob"),
        person(dir.path(), "eve"),
    );
    let pubfile = dir.path().join("bob.pub");
    fs::write(&pubfile, format!("# bob\n{}\n", bob.public)).unwrap();
    let input = sample_input(dir.path());
    let archive = dir.path().join("p.vqm");
    vaqum()
        .args([
            "compress",
            input.to_str().unwrap(),
            "--to",
            &alice.public,
            "--to",
            pubfile.to_str().unwrap(),
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("for 2 recipients"));
    for who in [&alice, &bob] {
        let out = dir.path().join(format!(
            "out-{}",
            who.identity.file_stem().unwrap().to_string_lossy()
        ));
        fs::create_dir_all(&out).unwrap();
        vaqum()
            .args([
                "decompress",
                archive.to_str().unwrap(),
                "-i",
                who.identity.to_str().unwrap(),
                "-o",
                out.to_str().unwrap(),
                "--verify",
            ])
            .assert()
            .success();
        assert_eq!(
            fs::read(&input).unwrap(),
            fs::read(out.join("plans.txt")).unwrap()
        );
    }
    vaqum()
        .args([
            "list",
            archive.to_str().unwrap(),
            "-i",
            eve.identity.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "wasn't encrypted for your key (2 recipients)",
        ));
}

#[test]
fn recipient_archive_rejects_header_tampering() {
    let dir = TempDir::new().unwrap();
    let bob = person(dir.path(), "bob");
    let input = sample_input(dir.path());
    let archive = dir.path().join("p.vqm");
    vaqum()
        .args([
            "compress",
            input.to_str().unwrap(),
            "--to",
            &bob.public,
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .success();
    let original = fs::read(&archive).unwrap();
    assert_eq!(original[4], 3, "recipient archives use format version 3");
    // Byte 8 starts the original-size field; byte 76 is inside the name.
    for (offset, what) in [(8usize, "size"), (75, "name")] {
        let mut bytes = original.clone();
        bytes[offset] ^= 0x01;
        fs::write(&archive, &bytes).unwrap();
        let out = vaqum()
            .args([
                "list",
                archive.to_str().unwrap(),
                "-i",
                bob.identity.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(!out.status.success(), "tampered {what} was accepted");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("tampered") || stderr.contains("plain file name"),
            "{what}: {stderr}"
        );
    }
}

#[test]
fn public_key_typos_are_caught() {
    let dir = TempDir::new().unwrap();
    let bob = person(dir.path(), "bob");
    let mut typo: Vec<char> = bob.public.chars().collect();
    let i = typo.len() / 2;
    typo[i] = if typo[i] == 'A' { 'B' } else { 'A' };
    let typo: String = typo.into_iter().collect();
    vaqum()
        .args([
            "compress",
            sample_input(dir.path()).to_str().unwrap(),
            "--to",
            &typo,
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("typo").or(predicate::str::contains("damaged")));
}

#[test]
fn signed_archive_verifies_and_tampering_or_wrong_signer_is_caught() {
    let dir = TempDir::new().unwrap();
    let (alice, mallory) = (person(dir.path(), "alice"), person(dir.path(), "mallory"));
    let input = sample_input(dir.path());
    let archive = dir.path().join("p.vqm");
    vaqum()
        .args([
            "compress",
            input.to_str().unwrap(),
            &format!("--sign={}", alice.identity.display()),
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("signature:"));
    assert!(dir.path().join("p.vqm.minisig").exists());

    vaqum()
        .args(["verify", archive.to_str().unwrap(), "--from", &alice.public])
        .assert()
        .success()
        .stdout(predicate::str::contains("Good signature"));
    vaqum()
        .args([
            "verify",
            archive.to_str().unwrap(),
            "--from",
            &mallory.public,
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("signature check FAILED"));

    let mut bytes = fs::read(&archive).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    fs::write(&archive, bytes).unwrap();
    vaqum()
        .args(["verify", archive.to_str().unwrap(), "--from", &alice.public])
        .assert()
        .failure()
        .stderr(predicate::str::contains("signature check FAILED"));
}

#[test]
fn decompress_from_refuses_unsigned_or_wrongly_signed_archives_before_extracting() {
    let dir = TempDir::new().unwrap();
    let (alice, mallory) = (person(dir.path(), "alice"), person(dir.path(), "mallory"));
    let archive = sample_archive(dir.path(), &[]);
    let out = dir.path().join("out");
    // No signature at all.
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "--from",
            &alice.public,
            "-o",
            out.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no readable signature"));
    assert!(
        !out.exists(),
        "nothing may be extracted before the signature checks out"
    );
    // Signed by someone else.
    vaqum()
        .args([
            "sign",
            archive.to_str().unwrap(),
            "-i",
            mallory.identity.to_str().unwrap(),
        ])
        .assert()
        .success();
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "--from",
            &alice.public,
            "-o",
            out.to_str().unwrap(),
        ])
        .assert()
        .failure();
    assert!(!out.exists());
    // Signed by the expected sender.
    vaqum()
        .args([
            "sign",
            archive.to_str().unwrap(),
            "-i",
            alice.identity.to_str().unwrap(),
        ])
        .assert()
        .success();
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "--from",
            &alice.public,
            "-o",
            out.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("Signature verified"));
    assert!(out.join("src_tree/a.txt").exists());
}

#[test]
fn recipients_and_password_options_cannot_be_mixed() {
    let dir = TempDir::new().unwrap();
    let bob = person(dir.path(), "bob");
    vaqum()
        .args([
            "compress",
            sample_input(dir.path()).to_str().unwrap(),
            "--to",
            &bob.public,
            "-e",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

// ---------------------------------------------------------------------
// large inputs: streaming paths
// ---------------------------------------------------------------------

#[test]
fn dedup_archive_extracts_from_stdin_too() {
    // Files go through the two-pass path; stdin can't be re-read, so it
    // takes the staging fallback. Both must produce the same tree.
    let dir = TempDir::new().unwrap();
    let archive = sample_archive(dir.path(), &["--dedup"]);
    let out = dir.path().join("out");
    vaqum()
        .args(["decompress", "-", "-o", out.to_str().unwrap(), "--verify"])
        .write_stdin(fs::read(&archive).unwrap())
        .assert()
        .success();
    assert_trees_equal(&dir.path().join("src_tree"), &out.join("src_tree"));
}

#[test]
fn dedup_archive_missing_a_content_object_is_reported() {
    let dir = TempDir::new().unwrap();
    // Manifest points at a well-formed hash the archive doesn't contain.
    let tar = dedup_tar_with("ok.txt", &"b".repeat(64), b"data");
    let archive = dir.path().join("broken.vqm");
    write_raw_archive(&archive, 1, 1, tar.len() as u64, "victim", &tar);
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            dir.path().join("out").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("missing"));
}

#[test]
fn verify_catches_a_checksum_mismatch_while_streaming() {
    let dir = TempDir::new().unwrap();
    let archive = sample_archive(dir.path(), &[]);
    let mut bytes = fs::read(&archive).unwrap();
    bytes[16] ^= 0xFF; // first byte of the stored SHA-256
    fs::write(&archive, bytes).unwrap();
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            dir.path().join("out").to_str().unwrap(),
            "--verify",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("checksum verification FAILED"))
        .stderr(predicate::str::contains("treat them as damaged"));
}

#[test]
fn search_streams_files_and_skips_pathological_lines() {
    let dir = TempDir::new().unwrap();
    let mut normal = fs::File::create(dir.path().join("log.txt")).unwrap();
    for i in 0..50_000 {
        writeln!(
            normal,
            "line {i} {}",
            if i == 31_337 {
                "NEEDLE here"
            } else {
                "nothing"
            }
        )
        .unwrap();
    }
    // A single 17 MiB line: treated as not-really-text instead of loaded.
    let mut blob = vec![b'x'; 17 * 1024 * 1024];
    blob.extend_from_slice(b"NEEDLE\n");
    fs::write(dir.path().join("minified.js"), blob).unwrap();
    vaqum()
        .args([
            "search",
            "NEEDLE",
            dir.path().to_str().unwrap(),
            "--content-only",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "log.txt:31338: line 31337 NEEDLE here",
        ))
        .stdout(predicate::str::contains("minified.js").not());
}

/// A file just over diff's line-by-line limit (64 MiB).
fn write_large(path: &std::path::Path, tail: &[u8]) {
    let mut f = fs::File::create(path).unwrap();
    let chunk = vec![b'a'; 1024 * 1024];
    for _ in 0..65 {
        f.write_all(&chunk).unwrap();
    }
    f.write_all(tail).unwrap();
}

#[test]
fn diff_compares_huge_files_by_checksum_without_loading_them() {
    let dir = TempDir::new().unwrap();
    let (a, b, c) = (
        dir.path().join("a.bin"),
        dir.path().join("b.bin"),
        dir.path().join("c.bin"),
    );
    write_large(&a, b"one");
    write_large(&b, b"one");
    write_large(&c, b"two");
    vaqum()
        .args(["diff", a.to_str().unwrap(), b.to_str().unwrap()])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("are identical"));
    vaqum()
        .args(["diff", a.to_str().unwrap(), c.to_str().unwrap()])
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "too large for a line-by-line diff",
        ));
}

#[test]
fn diff_of_folders_reports_huge_modified_files_without_reading_them() {
    let dir = TempDir::new().unwrap();
    let (x, y) = (dir.path().join("x"), dir.path().join("y"));
    fs::create_dir_all(&x).unwrap();
    fs::create_dir_all(&y).unwrap();
    write_large(&x.join("big.log"), b"old");
    write_large(&y.join("big.log"), b"new");
    vaqum()
        .args(["diff", x.to_str().unwrap(), y.to_str().unwrap()])
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "big.log  (too large for a line diff",
        ));
}

// ---------------------------------------------------------------------
// robustness
// ---------------------------------------------------------------------

#[test]
fn archive_saved_inside_the_folder_never_includes_itself() {
    for extra in [&[][..], &["--dedup"][..]] {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("proj");
        build_sample_tree(&src);
        let out = src.join("proj.vqm");
        fs::write(&out, "an older archive being replaced").unwrap();
        let mut args = vec![
            "compress",
            src.to_str().unwrap(),
            "-r",
            "-o",
            out.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        vaqum().args(&args).assert().success();

        let listing = list_json(&out, &[]);
        let paths = listed_paths(&listing);
        assert!(
            !paths
                .iter()
                .any(|p| p.ends_with(".vqm") || p.contains(".tmp")),
            "{extra:?}: {paths:?}"
        );
        let mut names: Vec<String> = fs::read_dir(&src)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["a.txt", "b.txt", "nested", "proj.vqm"],
            "{extra:?}: no temp files may remain"
        );
    }
}

#[cfg(unix)]
#[test]
fn dedupe_link_is_idempotent() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("photos");
    build_sample_tree(&src);
    vaqum()
        .args(["dedupe", src.to_str().unwrap(), "--link"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hardlinked 1 duplicate"));
    // Already linked: nothing to do, nothing counted as reclaimed again.
    vaqum()
        .args(["dedupe", src.to_str().unwrap(), "--link"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hardlinked 0 duplicate"));
    assert_eq!(
        fs::read(src.join("a.txt")).unwrap(),
        fs::read(src.join("b.txt")).unwrap()
    );
}

#[cfg(unix)]
#[test]
fn interrupting_compress_leaves_no_temp_files() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("big.bin");
    // xz at maximum is slow enough to interrupt reliably.
    let f = fs::File::create(&input).unwrap();
    f.set_len(400 * 1024 * 1024).unwrap();
    let bin = assert_cmd::cargo::cargo_bin("vaqum");
    let child = std::process::Command::new(bin)
        .args([
            "compress",
            input.to_str().unwrap(),
            "--max",
            "-q",
            "--no-secret-check",
        ])
        .env("XDG_CONFIG_HOME", isolated_config_home())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(800));
    std::process::Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(130),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let names: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["big.bin"], "temp files left behind: {names:?}");
}

// ---------------------------------------------------------------------
// integrity: byte-for-byte round trips over tricky inputs, every mode
// ---------------------------------------------------------------------

/// Deterministic pseudo-random bytes (incompressible), no extra deps.
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

/// A tree of edge cases: empty and 1-byte files, sizes straddling the
/// 64 KiB encryption chunk, every byte value, Unicode and spaces in
/// names, deep nesting, an empty directory, duplicates.
fn tricky_tree(root: &std::path::Path) {
    fs::create_dir_all(root.join("empty-dir")).unwrap();
    fs::write(root.join("empty.txt"), b"").unwrap();
    fs::write(root.join("one.bin"), b"\x00").unwrap();
    for (i, size) in [65_535usize, 65_536, 65_537, 131_072, 1_000_003]
        .iter()
        .enumerate()
    {
        fs::write(
            root.join(format!("noise-{size}.bin")),
            noise(*size, i as u64 + 1),
        )
        .unwrap();
    }
    fs::write(
        root.join("all-bytes.bin"),
        (0..=255u8).cycle().take(70_000).collect::<Vec<_>>(),
    )
    .unwrap();
    fs::write(root.join("résumé – 日本語 🎉.txt"), "unicode name\n").unwrap();
    fs::write(root.join("with spaces.txt"), "spaces\n").unwrap();
    let mut deep = root.to_path_buf();
    for level in 0..20 {
        deep = deep.join(format!("d{level}"));
    }
    fs::create_dir_all(&deep).unwrap();
    fs::write(deep.join("bottom.txt"), "deep\n").unwrap();
    let text: String = (0..80_000)
        .map(|i| format!("log {i} status=200\n"))
        .collect();
    fs::write(root.join("big.log"), &text).unwrap();
    fs::write(root.join("big-copy.log"), &text).unwrap();
}

/// Every directory (including empty ones) under `root`, relative.
fn all_dirs(root: &std::path::Path) -> Vec<String> {
    let mut dirs: Vec<String> = walkdir::WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_dir())
        .map(|e| {
            e.path()
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn every_mode_round_trips_tricky_inputs_byte_for_byte() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("tricky");
    tricky_tree(&src);
    let key = dir.path().join("key");
    fs::write(&key, "integrity key").unwrap();
    let identity = dir.path().join("me.key");
    vaqum()
        .args(["keygen", "-o", identity.to_str().unwrap()])
        .assert()
        .success();
    let me = String::from_utf8(
        vaqum()
            .args(["pubkey", "-i", identity.to_str().unwrap()])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let me = me.trim();

    let modes: Vec<(&str, Vec<&str>, Vec<&str>)> = vec![
        ("plain", vec![], vec![]),
        ("xz", vec!["--max"], vec![]),
        ("dedup", vec!["--dedup"], vec![]),
        ("level 1", vec!["-l", "1"], vec![]),
        ("level 19", vec!["-l", "19"], vec![]),
        (
            "password",
            vec!["--key-file", key.to_str().unwrap()],
            vec!["--key-file", key.to_str().unwrap()],
        ),
        (
            "dedup + password",
            vec!["--dedup", "--key-file", key.to_str().unwrap()],
            vec!["--key-file", key.to_str().unwrap()],
        ),
        (
            "recipient",
            vec!["--to", me],
            vec!["-i", identity.to_str().unwrap()],
        ),
        (
            "xz + recipient",
            vec!["--max", "--to", me],
            vec!["-i", identity.to_str().unwrap()],
        ),
    ];
    for (i, (label, compress_flags, open_flags)) in modes.iter().enumerate() {
        let archive = dir.path().join(format!("m{i}.vqm"));
        let mut args = vec![
            "compress",
            src.to_str().unwrap(),
            "-r",
            "-q",
            "--no-secret-check",
            "-o",
            archive.to_str().unwrap(),
        ];
        args.extend(compress_flags);
        vaqum().args(&args).assert().success();
        let out = dir.path().join(format!("out{i}"));
        let mut args = vec![
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--verify",
            "-q",
        ];
        args.extend(open_flags);
        vaqum().args(&args).assert().success();
        assert_trees_equal(&src, &out.join("tricky"));
        assert_eq!(
            all_dirs(&src),
            all_dirs(&out.join("tricky")),
            "{label}: directories (incl. empty) must survive"
        );
        // --only through the same archive.
        let only = dir.path().join(format!("only{i}"));
        let mut args = vec![
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            only.to_str().unwrap(),
            "--only",
            "*.bin",
            "-q",
        ];
        args.extend(open_flags);
        vaqum().args(&args).assert().success();
        let mut got = walk_relative(&only.join("tricky"));
        got.sort();
        let mut want: Vec<String> = walk_relative(&src)
            .into_iter()
            .filter(|p| p.ends_with(".bin"))
            .collect();
        want.sort();
        assert_eq!(got, want, "{label}: --only *.bin");
        for rel in &want {
            assert_eq!(
                fs::read(src.join(rel)).unwrap(),
                fs::read(only.join("tricky").join(rel)).unwrap(),
                "{label}: {rel}"
            );
        }
    }
}

#[test]
fn single_file_archives_round_trip_edge_sizes_including_empty() {
    let dir = TempDir::new().unwrap();
    let key = dir.path().join("key");
    fs::write(&key, "k").unwrap();
    for (i, data) in [
        vec![],
        vec![7u8],
        noise(65_536, 9),
        noise(65_537, 10),
        noise(200_000, 11),
    ]
    .iter()
    .enumerate()
    {
        let input = dir.path().join(format!("f{i}.bin"));
        fs::write(&input, data).unwrap();
        for extra in [
            &[][..],
            &["--max"][..],
            &["--key-file", key.to_str().unwrap()][..],
        ] {
            let archive = dir.path().join(format!("f{i}-{}.vqm", extra.len()));
            let mut args = vec![
                "compress",
                input.to_str().unwrap(),
                "-q",
                "-o",
                archive.to_str().unwrap(),
            ];
            args.extend_from_slice(extra);
            vaqum().args(&args).assert().success();
            let opened = vaqum()
                .args([
                    "decompress",
                    archive.to_str().unwrap(),
                    "-o",
                    "-",
                    "--verify",
                ])
                .args(if extra.contains(&"--key-file") {
                    vec!["--key-file", key.to_str().unwrap()]
                } else {
                    vec![]
                })
                .output()
                .unwrap();
            assert!(
                opened.status.success(),
                "{i} {extra:?}: {}",
                String::from_utf8_lossy(&opened.stderr)
            );
            assert_eq!(&opened.stdout, data, "{i} {extra:?}: bytes differ");
        }
    }
}

#[test]
fn every_corrupted_byte_in_a_plain_archive_is_detected_or_harmless() {
    // Flip each byte of a small archive in turn: --verify must either
    // fail or produce the exact original. Never silently wrong output.
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("data.txt");
    write_sample_text(&input, 40);
    let archive = dir.path().join("data.vqm");
    vaqum()
        .args([
            "compress",
            input.to_str().unwrap(),
            "-q",
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .success();
    let original = fs::read(&archive).unwrap();
    let expected = fs::read(&input).unwrap();
    // A key is always supplied: when corruption flips the "encrypted" flag,
    // vaqum tries (and fails) to decrypt instead of asking for a password.
    let key = dir.path().join("key");
    fs::write(&key, "any key").unwrap();
    for pos in 0..original.len() {
        let mut bytes = original.clone();
        bytes[pos] ^= 0x5A;
        fs::write(&archive, &bytes).unwrap();
        let out = vaqum()
            .args([
                "decompress",
                archive.to_str().unwrap(),
                "-o",
                "-",
                "--verify",
                "--key-file",
                key.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        if out.status.success() {
            assert_eq!(
                out.stdout, expected,
                "byte {pos}: corruption produced wrong output that passed --verify"
            );
        }
    }
}

#[test]
fn encrypted_archive_without_a_terminal_fails_fast_instead_of_waiting() {
    // No terminal (as in scripts and CI): asking for a password must fail at
    // once with guidance. On Windows a console prompt used to wait forever.
    let dir = TempDir::new().unwrap();
    let key = dir.path().join("k");
    fs::write(&key, "k").unwrap();
    let input = dir.path().join("s.txt");
    fs::write(&input, "secret").unwrap();
    let archive = dir.path().join("s.vqm");
    vaqum()
        .args([
            "compress",
            input.to_str().unwrap(),
            "--key-file",
            key.to_str().unwrap(),
            "-o",
            archive.to_str().unwrap(),
        ])
        .assert()
        .success();
    let started = std::time::Instant::now();
    vaqum()
        .args([
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            dir.path().join("out").to_str().unwrap(),
        ])
        .timeout(std::time::Duration::from_secs(20))
        .assert()
        .failure()
        .stderr(predicate::str::contains("no terminal to type it in"))
        .stderr(predicate::str::contains("--key-file"));
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    vaqum()
        .args(["compress", input.to_str().unwrap(), "-e"])
        .timeout(std::time::Duration::from_secs(20))
        .assert()
        .failure()
        .stderr(predicate::str::contains("no terminal to type it in"));
}

/// Runs vaqum attached to a pseudo-terminal (its controlling terminal, as
/// in a real shell) and types each answer when a "password:" prompt shows.
/// A watchdog kills it after 30 s so a regression fails instead of hanging.
#[cfg(unix)]
fn run_in_terminal(args: &[&str], answers: &[&str]) -> (Option<i32>, String) {
    use std::io::{Read, Write};
    use std::os::unix::process::CommandExt;

    let pty = nix::pty::openpty(None, None).expect("openpty");
    let slave = std::fs::File::from(pty.slave);
    let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin("vaqum"));
    cmd.args(args)
        .env("XDG_CONFIG_HOME", isolated_config_home())
        .stdin(slave.try_clone().unwrap())
        .stdout(slave.try_clone().unwrap())
        .stderr(slave);
    unsafe {
        cmd.pre_exec(|| {
            nix::unistd::setsid().map_err(std::io::Error::from)?;
            if nix::libc::ioctl(0, nix::libc::TIOCSCTTY as _, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().expect("spawn vaqum");
    drop(cmd); // close our copies of the terminal's slave side

    let pid = nix::unistd::Pid::from_raw(child.id() as i32);
    let watchdog = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(30));
        let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
    });

    let mut master = std::fs::File::from(pty.master);
    let mut seen = Vec::new();
    let mut answered = 0;
    let mut buf = [0u8; 4096];
    loop {
        match master.read(&mut buf) {
            Ok(0) | Err(_) => break, // the child closed the terminal
            Ok(n) => seen.extend_from_slice(&buf[..n]),
        }
        let last_line = seen.rsplit(|&b| b == b'\n').next().unwrap_or_default();
        if answered < answers.len()
            && String::from_utf8_lossy(last_line)
                .to_lowercase()
                .contains("password:")
        {
            master
                .write_all(format!("{}\n", answers[answered]).as_bytes())
                .unwrap();
            answered += 1;
        }
    }
    let status = child.wait().unwrap();
    drop(watchdog);
    (status.code(), String::from_utf8_lossy(&seen).into_owned())
}

/// Windows version of [`run_in_terminal`], through a ConPTY pseudo-console
/// (what Windows Terminal uses). Enter is `\r`; the output stream doesn't
/// end when the child exits, so a reader thread feeds a channel and the
/// loop watches the process instead. A 60 s deadline kills a stuck child.
#[cfg(windows)]
fn run_in_terminal(args: &[&str], answers: &[&str]) -> (Option<i32>, String) {
    use std::io::{Read, Write};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 40,
            cols: 200,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open a pseudo-console");
    let mut cmd = CommandBuilder::new(assert_cmd::cargo::cargo_bin("vaqum"));
    cmd.args(args);
    cmd.env("APPDATA", isolated_config_home());
    cmd.cwd(std::env::current_dir().unwrap());
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .expect("spawn vaqum in a pseudo-console");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let deadline = Instant::now() + Duration::from_secs(60);
    let mut seen = Vec::new();
    let (mut answered, mut cursor_replies) = (0, 0);
    let status = loop {
        while let Ok(chunk) = rx.recv_timeout(Duration::from_millis(100)) {
            seen.extend(chunk);
        }
        let text = String::from_utf8_lossy(&seen).to_lowercase();
        // ConPTY may ask where the cursor is and wait for an answer.
        let asks = text.matches("\x1b[6n").count();
        while cursor_replies < asks {
            writer.write_all(b"\x1b[1;1R").unwrap();
            cursor_replies += 1;
        }
        let prompts = text.matches("password:").count();
        while answered < answers.len() && answered < prompts {
            writer
                .write_all(format!("{}\r", answers[answered]).as_bytes())
                .unwrap();
            writer.flush().unwrap();
            answered += 1;
        }
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status.exit_code() as i32);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            break None;
        }
    };
    drop(writer);
    drop(pair.master);
    while let Ok(chunk) = rx.recv_timeout(Duration::from_millis(300)) {
        seen.extend(chunk);
    }
    (status, String::from_utf8_lossy(&seen).into_owned())
}

#[test]
fn typed_password_at_a_terminal_encrypts_and_decrypts() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("secret.txt");
    fs::write(&input, "typed at a terminal\n").unwrap();
    let archive = dir.path().join("secret.vqm");
    let out = dir.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let password = "Correct-Horse-Battery-9";

    let (code, transcript) = run_in_terminal(
        &[
            "compress",
            input.to_str().unwrap(),
            "-e",
            "-o",
            archive.to_str().unwrap(),
            "--no-secret-check",
        ],
        &[password, password],
    );
    assert_eq!(code, Some(0), "encrypt failed:\n{transcript}");
    assert!(transcript.contains("Confirm password"), "{transcript}");

    let (code, transcript) = run_in_terminal(
        &[
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ],
        &["wrong password"],
    );
    assert_ne!(
        code,
        Some(0),
        "a wrong typed password was accepted:\n{transcript}"
    );

    let (code, transcript) = run_in_terminal(
        &[
            "decompress",
            archive.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ],
        &[password],
    );
    assert_eq!(code, Some(0), "decrypt failed:\n{transcript}");
    assert_eq!(
        fs::read_to_string(out.join("secret.txt")).unwrap(),
        "typed at a terminal\n"
    );
}
