//! Secrets files (`vaqum secrets`): arbitrary text must never crash the
//! parser, and whatever names/values go in must come back out exactly.
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use vaqum::keys::Identity;
use vaqum::secret_store::SecretsFile;

fn identity() -> &'static Identity {
    static ID: OnceLock<Identity> = OnceLock::new();
    ID.get_or_init(|| Identity::generate().expect("generate a key"))
}

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);

    // 1. Hostile input: parse, list, try to unlock. No panics.
    if let Ok(file) = SecretsFile::parse(&text) {
        let _ = file.names().count();
        let _ = file.unlock(identity());
    }

    // 2. Round trip: fuzz-chosen names and values survive save + reload.
    let mut store = SecretsFile::create(std::slice::from_ref(identity().public())).unwrap();
    let mut expected = Vec::new();
    for (i, chunk) in text.split('\u{1}').take(8).enumerate() {
        let name = format!("K{i}");
        let value: String = chunk.chars().filter(|c| *c != '\0').collect();
        store.set(&name, &value).unwrap();
        expected.push((name, value));
    }
    let rendered = store.render();
    let reopened = SecretsFile::parse(&rendered).unwrap().unlock(identity()).unwrap();
    assert_eq!(reopened.all().unwrap(), expected);
});
