//! Feeds arbitrary bytes to vaqum as a .vqm archive: header parsing,
//! listing, and extraction must never panic, hang on a bomb, or write
//! outside the output directory.
#![no_main]

use libfuzzer_sys::fuzz_target;
use vaqum::{archive, dedup, extract::Extracted, format::EntryType, format::Header};

mod common;

fuzz_target!(|data: &[u8]| {
    let mut rest = data;
    let Ok(header) = Header::read(&mut rest) else {
        return;
    };
    // Encrypted archives need a password; huge claimed sizes just make the
    // fuzzer slow (the size limit itself is covered by the test suite).
    if header.encrypted || header.original_size > 4 << 20 {
        return;
    }

    if let Ok(payload) = archive::decoded_payload(rest, &header, None) {
        let _ = archive::list_vaqum(payload, &header);
    }

    let Ok(mut payload) = archive::decoded_payload(rest, &header, None) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("out");
    match header.entry_type {
        EntryType::File => {
            let _ = std::io::copy(&mut payload, &mut std::io::sink());
        }
        EntryType::Archive | EntryType::Bundle => {
            let mut ext = Extracted::default();
            let staging = dir.path().join("staging");
            let _ = dedup::unpack_tar(payload, header.dedup, &staging, &dest, "", &mut ext);
            common::assert_contained(dir.path(), &dest, &ext);
        }
    }
});
