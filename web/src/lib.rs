//! vaqum archive viewer for the browser (WebAssembly).
//!
//! Opens `.vqm` archives (plain or password-encrypted) entirely in memory:
//! nothing is uploaded anywhere. It reuses vaqum's own header parser and
//! risk checks (`src/format.rs`, `src/risk.rs`) so the viewer can't drift
//! from the CLI, and pure-Rust decoders (ruzstd, lzma-rs) since vaqum's C
//! codecs don't build for the browser.
//!
//! The JavaScript side (`site/viewer.js`) talks to it through a few plain
//! exported functions and JSON.

use std::io::{self, Read, Write};
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use argon2::Argon2;
use chacha20poly1305::ChaCha20Poly1305;
use chacha20poly1305::aead::stream::DecryptorBE32;
use serde::Serialize;
use sha2::{Digest, Sha256};

#[path = "../../src/format.rs"]
#[allow(dead_code)]
mod format;
#[path = "../../src/risk.rs"]
#[allow(dead_code)]
mod risk;

/// Constants `format.rs` expects from the main crate's other modules.
mod crypto {
    pub const SALT_LEN: usize = 16;
    pub const NONCE_PREFIX_LEN: usize = 7;
}
mod keys {
    pub const STANZA_LEN: usize = 80;
    pub const MAC_LEN: usize = 32;
    pub type Stanza = [u8; STANZA_LEN];
}

use format::{Algorithm, EntryType, Header};

/// Browsers can't hold much more than this in memory comfortably.
const MAX_OUTPUT: u64 = 1536 * 1024 * 1024;
const CHUNK_SIZE: usize = 64 * 1024;
const TAG_LEN: usize = 16;
const MANIFEST_ENTRY: &str = ".vaqum-manifest.json";
const OBJECTS_DIR: &str = ".vaqum-objects/";

// ------------------------------------------------------------------ model

#[derive(Serialize)]
pub struct EntryInfo {
    pub path: String,
    pub kind: &'static str,
    pub size: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    pub name: String,
    pub kind: &'static str,
    pub algorithm: &'static str,
    pub encrypted: bool,
    pub original_size: u64,
    pub verified: bool,
    pub entries: Vec<EntryInfo>,
}

/// Why an archive couldn't be opened (yet).
#[derive(Debug)]
pub enum OpenError {
    NeedsPassword,
    WrongPassword,
    Recipients,
    Other(String),
}

struct File {
    path: String,
    data: Vec<u8>,
}

// ------------------------------------------------------------- decryption

fn derive_key(password: &[u8], salt: &[u8; 16], strong: bool) -> Result<chacha20poly1305::Key> {
    // Must match vaqum's crypto::derive_key.
    let argon = if strong {
        let params =
            argon2::Params::new(64 * 1024, 3, 1, Some(32)).map_err(|e| anyhow::anyhow!("{e}"))?;
        Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
    } else {
        Argon2::default()
    };
    let mut key = [0u8; 32];
    argon
        .hash_password_into(password, salt, &mut key)
        .map_err(|e| anyhow::anyhow!("key derivation failed: {e}"))?;
    Ok(key.into())
}

/// Reverses vaqum's chunked ChaCha20-Poly1305 STREAM encryption.
fn decrypt(data: &[u8], key: &chacha20poly1305::Key, nonce_prefix: &[u8; 7]) -> Option<Vec<u8>> {
    let mut stream = DecryptorBE32::<ChaCha20Poly1305>::new(key, nonce_prefix.into());
    let mut out = Vec::with_capacity(data.len());
    let mut chunks = data.chunks(CHUNK_SIZE + TAG_LEN).peekable();
    chunks.peek()?;
    while let Some(chunk) = chunks.next() {
        if chunks.peek().is_some() {
            out.extend(stream.decrypt_next(chunk).ok()?);
        } else {
            out.extend(stream.decrypt_last(chunk).ok()?);
            return Some(out);
        }
    }
    None
}

// ---------------------------------------------------------- decompression

/// A writer that refuses to grow past `limit` (decompression-bomb guard).
struct Limited {
    buf: Vec<u8>,
    limit: u64,
}

impl Write for Limited {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.buf.len() as u64 + data.len() as u64 > self.limit {
            return Err(io::Error::other(
                "archive decompresses to more data than its header claims (possible decompression bomb)",
            ));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn decompress(data: &[u8], algorithm: Algorithm, limit: u64) -> Result<Vec<u8>> {
    let mut out = Limited {
        buf: Vec::new(),
        limit,
    };
    match algorithm {
        Algorithm::Zstd => {
            let mut input = data;
            // A zstd stream can hold several frames back to back.
            while !input.is_empty() {
                let mut frame = ruzstd::decoding::StreamingDecoder::new(&mut input)
                    .map_err(|e| anyhow::anyhow!("corrupt compressed data: {e}"))?;
                io::copy(&mut frame, &mut out).context("corrupt compressed data")?;
            }
        }
        Algorithm::Xz => {
            lzma_rs::xz_decompress(&mut io::BufReader::new(data), &mut out)
                .map_err(|e| anyhow::anyhow!("corrupt compressed data: {e:?}"))?;
        }
    }
    Ok(out.buf)
}

// ------------------------------------------------------------------ paths

fn normalize(path: &str) -> String {
    let mut s = path.replace('\\', "/");
    while let Some(rest) = s.strip_prefix("./").or_else(|| s.strip_prefix('/')) {
        s = rest.to_string();
    }
    s.trim_end_matches('/').to_string()
}

fn is_safe(path: &str) -> bool {
    !path.is_empty()
        && path
            .split('/')
            .all(|c| !c.is_empty() && c != "." && c != "..")
}

fn head(data: &[u8]) -> &[u8] {
    &data[..data.len().min(risk::HEAD_LEN)]
}

// ------------------------------------------------------------------ core

/// Opens an archive held in memory, returning its contents.
fn open(
    bytes: &[u8],
    password: Option<&[u8]>,
) -> std::result::Result<(Opened, Vec<File>), OpenError> {
    let other = |e: anyhow::Error| OpenError::Other(format!("{e:#}"));
    let mut rest = bytes;
    let header = Header::read(&mut rest).map_err(other)?;
    if !header.recipients.is_empty() {
        return Err(OpenError::Recipients);
    }
    if header.original_size > MAX_OUTPUT {
        return Err(OpenError::Other(
            "this archive is too large to open in a browser; use the vaqum command-line tool"
                .into(),
        ));
    }

    let compressed = if header.encrypted {
        let Some(password) = password.filter(|p| !p.is_empty()) else {
            return Err(OpenError::NeedsPassword);
        };
        let key = derive_key(password, &header.salt, header.strong_kdf).map_err(other)?;
        decrypt(rest, &key, &header.nonce_prefix).ok_or(OpenError::WrongPassword)?
    } else {
        rest.to_vec()
    };
    let payload = decompress(&compressed, header.algorithm, header.original_size).map_err(other)?;
    let verified = payload.len() as u64 == header.original_size
        && Sha256::digest(&payload).as_slice() == header.checksum;
    if !verified {
        return Err(OpenError::Other(
            "checksum mismatch: the archive is corrupted".into(),
        ));
    }

    let (files, entries) = match header.entry_type {
        EntryType::File => {
            let warnings = risk::assess(&header.name, risk::Kind::File, None, head(&payload));
            let entry = EntryInfo {
                path: header.name.clone(),
                kind: "file",
                size: payload.len() as u64,
                warnings,
            };
            (
                vec![File {
                    path: header.name.clone(),
                    data: payload,
                }],
                vec![entry],
            )
        }
        EntryType::Archive | EntryType::Bundle => {
            let prefix = match header.entry_type {
                EntryType::Archive => format!("{}/", header.name),
                _ => String::new(),
            };
            read_tar(&payload, header.dedup, &prefix).map_err(other)?
        }
    };

    let opened = Opened {
        name: header.name.clone(),
        kind: match header.entry_type {
            EntryType::File => "single file",
            EntryType::Archive => "directory archive",
            EntryType::Bundle => "multi-path bundle",
        },
        algorithm: header.algorithm.label(),
        encrypted: header.encrypted,
        original_size: header.original_size,
        verified,
        entries,
    };
    Ok((opened, files))
}

fn read_tar(payload: &[u8], dedup: bool, prefix: &str) -> Result<(Vec<File>, Vec<EntryInfo>)> {
    let mut files = Vec::new();
    let mut entries = Vec::new();
    let mut objects: std::collections::HashMap<String, Vec<u8>> = Default::default();
    let mut manifest: Option<serde_json::Value> = None;

    let mut archive = tar::Archive::new(payload);
    for entry in archive
        .entries()
        .context("failed to read archive contents")?
    {
        let mut entry = entry.context("failed to read an archive entry")?;
        let rel = normalize(&entry.path()?.to_string_lossy());
        let etype = entry.header().entry_type();
        let mode = entry.header().mode().ok();

        if dedup {
            let mut data = Vec::new();
            entry.read_to_end(&mut data)?;
            if rel == MANIFEST_ENTRY {
                manifest = Some(serde_json::from_slice(&data).context("corrupt dedup manifest")?);
            } else if let Some(hash) = rel.strip_prefix(OBJECTS_DIR) {
                objects.insert(hash.to_string(), data);
            }
            continue;
        }
        if rel.is_empty() {
            continue;
        }
        let shown = format!("{prefix}{rel}");
        if !is_safe(&rel) {
            entries.push(EntryInfo {
                path: shown,
                kind: "file",
                size: 0,
                warnings: vec!["has an unsafe path (points outside the archive); skipped".into()],
            });
            continue;
        }
        if etype.is_dir() {
            entries.push(EntryInfo {
                warnings: risk::assess(&shown, risk::Kind::Dir, mode, &[]),
                path: shown,
                kind: "dir",
                size: 0,
            });
        } else if etype.is_file() {
            let mut data = Vec::new();
            entry.read_to_end(&mut data)?;
            entries.push(EntryInfo {
                warnings: risk::assess(&shown, risk::Kind::File, mode, head(&data)),
                path: shown.clone(),
                kind: "file",
                size: data.len() as u64,
            });
            files.push(File { path: shown, data });
        } else if etype.is_symlink() || etype.is_hard_link() {
            let target = entry
                .link_name()
                .ok()
                .flatten()
                .map(|t| t.to_string_lossy().into_owned())
                .unwrap_or_default();
            let kind = if etype.is_symlink() {
                risk::Kind::Symlink(&target)
            } else {
                risk::Kind::Hardlink(&target)
            };
            let mut warnings = risk::assess(&shown, kind, mode, &[]);
            warnings.push(format!(
                "is a link to {target}; links aren't downloadable here"
            ));
            entries.push(EntryInfo {
                path: shown,
                kind: "link",
                size: 0,
                warnings,
            });
        }
    }

    if dedup {
        let manifest = manifest.context("dedup archive is missing its manifest")?;
        for dir in manifest["dirs"].as_array().into_iter().flatten() {
            let rel = normalize(dir.as_str().unwrap_or_default());
            if is_safe(&rel) {
                entries.push(EntryInfo {
                    path: format!("{prefix}{rel}"),
                    kind: "dir",
                    size: 0,
                    warnings: Vec::new(),
                });
            }
        }
        for f in manifest["files"].as_array().into_iter().flatten() {
            let rel = normalize(f["path"].as_str().unwrap_or_default());
            let hash = f["hash"].as_str().unwrap_or_default();
            let mode = f["mode"].as_u64().map(|m| m as u32);
            let shown = format!("{prefix}{rel}");
            let Some(data) = objects.get(hash).filter(|_| is_safe(&rel)) else {
                bail!("corrupt dedup archive: bad entry for {rel}");
            };
            entries.push(EntryInfo {
                warnings: risk::assess(&shown, risk::Kind::File, mode, head(data)),
                path: shown.clone(),
                kind: "file",
                size: data.len() as u64,
            });
            files.push(File {
                path: shown,
                data: data.clone(),
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
    }
    Ok((files, entries))
}

/// Writes a plain .zip (deflate, no zip64: the viewer's size cap keeps
/// everything well under 4 GB) so "download all" is one file.
fn zip_all(files: &[File]) -> Result<Vec<u8>> {
    if files.len() > u16::MAX as usize {
        bail!("too many files for one zip; use the vaqum command-line tool");
    }
    let mut out = Vec::new();
    let mut central = Vec::new();
    for f in files {
        let name = f.path.as_bytes();
        let crc = crc32fast::hash(&f.data);
        let packed = miniz_oxide::deflate::compress_to_vec(&f.data, 6);
        let (method, body): (u16, &[u8]) = if packed.len() < f.data.len() {
            (8, &packed)
        } else {
            (0, &f.data)
        };
        let offset = u32::try_from(out.len()).context("zip too large")?;
        let (comp, size) = (body.len() as u32, f.data.len() as u32);
        // Local file header. Flag bit 11: names are UTF-8.
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        for v in [20u16, 0x0800, method, 0, 0x21] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in [crc, comp, size] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(body);
        // Central directory record.
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        for v in [20u16, 20, 0x0800, method, 0, 0x21] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        for v in [crc, comp, size] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        for v in [name.len() as u16, 0, 0, 0, 0] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }
    let central_offset = u32::try_from(out.len()).context("zip too large")?;
    let central_len = central.len() as u32;
    out.extend(central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    for v in [0u16, 0, files.len() as u16, files.len() as u16] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&central_len.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    Ok(out)
}

// ------------------------------------------------- WebAssembly interface

struct State {
    files: Vec<File>,
    out: Vec<u8>,
}

static STATE: Mutex<State> = Mutex::new(State {
    files: Vec::new(),
    out: Vec::new(),
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Allocates `len` bytes for the JavaScript side to fill.
#[unsafe(no_mangle)]
pub extern "C" fn vq_alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len.max(1));
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// Frees memory from [`vq_alloc`].
///
/// # Safety
/// `ptr`/`len` must come from one `vq_alloc` call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vq_free(ptr: *mut u8, len: usize) {
    drop(unsafe { Vec::from_raw_parts(ptr, 0, len.max(1)) });
}

/// Where the latest result (JSON or file bytes) is.
#[unsafe(no_mangle)]
pub extern "C" fn vq_out_ptr() -> *const u8 {
    state().out.as_ptr()
}

/// Opens the archive at `ptr..ptr+len` (password optional, may be empty)
/// and returns the length of a JSON result at [`vq_out_ptr`]:
/// `{"ok":true,...}` or `{"ok":false,"error":"...","needsPassword":bool}`.
///
/// # Safety
/// The pointers must be valid for their lengths.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vq_open(
    ptr: *const u8,
    len: usize,
    pw_ptr: *const u8,
    pw_len: usize,
) -> usize {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let password = (pw_len > 0).then(|| unsafe { std::slice::from_raw_parts(pw_ptr, pw_len) });
    let result = match open(bytes, password) {
        Ok((opened, files)) => {
            state().files = files;
            let mut v = serde_json::to_value(&opened).unwrap_or_default();
            v["ok"] = true.into();
            v
        }
        Err(e) => {
            state().files.clear();
            let (needs_password, error) = match e {
                OpenError::NeedsPassword => (true, "This archive is password-protected.".to_string()),
                OpenError::WrongPassword => (
                    true,
                    "Wrong password, or the archive is damaged.".to_string(),
                ),
                OpenError::Recipients => (
                    false,
                    "This archive was encrypted for specific people's keys. Open it with the vaqum command-line tool: vaqum decompress <file>".to_string(),
                ),
                OpenError::Other(msg) => (false, msg),
            };
            serde_json::json!({ "ok": false, "error": error, "needsPassword": needs_password })
        }
    };
    let mut st = state();
    st.out = serde_json::to_vec(&result).unwrap_or_default();
    st.out.len()
}

/// Puts file `index` (in download order) at [`vq_out_ptr`]; returns its
/// length, or `usize::MAX` if there's no such file.
#[unsafe(no_mangle)]
pub extern "C" fn vq_file(index: usize) -> usize {
    let mut st = state();
    let Some(data) = st.files.get(index).map(|f| f.data.clone()) else {
        return usize::MAX;
    };
    st.out = data;
    st.out.len()
}

/// Puts the path of file `index` at [`vq_out_ptr`].
#[unsafe(no_mangle)]
pub extern "C" fn vq_file_path(index: usize) -> usize {
    let mut st = state();
    let Some(path) = st.files.get(index).map(|f| f.path.clone().into_bytes()) else {
        return usize::MAX;
    };
    st.out = path;
    st.out.len()
}

/// Number of downloadable files in the opened archive.
#[unsafe(no_mangle)]
pub extern "C" fn vq_file_count() -> usize {
    state().files.len()
}

/// Puts a .zip of every file at [`vq_out_ptr`]; returns its length
/// (`usize::MAX` on failure).
#[unsafe(no_mangle)]
pub extern "C" fn vq_zip_all() -> usize {
    let mut st = state();
    match zip_all(&st.files) {
        Ok(zip) => {
            st.out = zip;
            st.out.len()
        }
        Err(_) => usize::MAX,
    }
}
