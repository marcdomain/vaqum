//! Arbitrary names, patterns and file starts through the `--only` matcher
//! and the risk checks: they must never panic.
#![no_main]

use libfuzzer_sys::fuzz_target;
use vaqum::archive::{self, Selectors};
use vaqum::format::{Algorithm, EntryType, Header};
use vaqum::risk;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let mut parts = text.splitn(3, '\n');
    let pattern = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default();
    let head = parts.next().unwrap_or_default().as_bytes();

    let header = Header {
        entry_type: EntryType::Archive,
        algorithm: Algorithm::Zstd,
        dedup: false,
        encrypted: false,
        strong_kdf: false,
        original_size: 0,
        checksum: [0; 32],
        salt: [0; 16],
        nonce_prefix: [0; 7],
        name: "root".into(),
        recipients: Vec::new(),
        mac: [0; 32],
    };
    if let Ok(mut selectors) = Selectors::new(&[pattern]) {
        selectors.select(&header, path);
        let _ = selectors.ensure_all_matched();
    }
    let _ = archive::normalize(path);
    let _ = archive::is_safe_relative(path);
    let _ = risk::assess(path, risk::Kind::File, Some(0o4755), head);
    let _ = risk::assess(path, risk::Kind::Symlink(path), None, &[]);
});
