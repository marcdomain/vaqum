//! Feeds arbitrary bytes as a tar stream to the extractor shared by .vqm,
//! `--only` and tar.gz. Nothing may be written outside the output dir.
#![no_main]

use libfuzzer_sys::fuzz_target;
use vaqum::extract::{self, Extracted};

mod common;

fuzz_target!(|data: &[u8]| {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("out");
    let mut ext = Extracted::default();
    let _ = extract::unpack_tar(data, &dest, "", &mut |_| true, &mut ext);
    common::assert_contained(dir.path(), &dest, &ext);
});
