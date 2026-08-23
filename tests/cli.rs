//! End-to-end tests that drive the real `vaqum` binary, the same way a
//! user would from a shell. Run with `cargo test`.

use std::fs;
use std::io::Write;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn vaqum() -> Command {
    Command::cargo_bin("vaqum").expect("vaqum binary should build")
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

    let archive = dir.path().join("original.txt.vaqum");
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
    let archive = dir.path().join("out.vaqum");

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
        !dir.path().join("original.txt.vaqum").exists(),
        "--dry-run must not write an output file"
    );
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

    let archive = dir.path().join("tree.vaqum");
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

    let plain_archive = dir.path().join("plain.vaqum");
    let dedup_archive = dir.path().join("dedup.vaqum");

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
                .to_string_lossy()
                .into_owned()
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
    let archive = dir.path().join("original.txt.vaqum");

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
    let bogus = dir.path().join("bogus.vaqum");
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
    let archive = dir.path().join("original.txt.vaqum");
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
    let archive = dir.path().join("tree.vaqum");
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
    let archive = dir.path().join("original.txt.vaqum");
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
    // The archive side must NOT be opened at its own .vaqum path (that's
    // compressed bytes, not text) — it should be a materialized scratch
    // file elsewhere.
    assert!(!log_contents.contains(archive.to_str().unwrap()));
    assert!(log_contents.contains(changed.to_str().unwrap()));
}

// ---------------------------------------------------------------------
// dedupe
// ---------------------------------------------------------------------

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
