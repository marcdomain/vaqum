# vaqum archive viewer

A web page that opens `.vqm` archives in the browser: plain or
password-protected, with risk warnings and per-file or zip downloads.
Nothing is uploaded; decryption and extraction run on the visitor's device
(WebAssembly), and the page's Content-Security-Policy blocks it from
loading or sending anything elsewhere.

It reuses vaqum's own header parser and risk checks (`../src/format.rs`,
`../src/risk.rs`) and pure-Rust decoders (ruzstd, lzma-rs), since vaqum's C
codecs don't build for the browser. Archives encrypted for specific people
(`--to`) need the command-line tool.

## Build and run locally

```sh
rustup target add wasm32-unknown-unknown
cd web
cargo build --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/vaqum_viewer.wasm site/vaqum-viewer.wasm
python3 -m http.server -d site 8000      # then open http://localhost:8000
```

## Test

```sh
cargo build                                    # from the repo root
VAQUM_BIN=target/debug/vaqum node --test web/test/viewer.test.mjs
```

The tests open archives made by the real CLI (every format variant,
password and legacy encryption, hostile input) through the same loader the
page uses.

## Publish

`.github/workflows/pages.yml` builds, tests and deploys `site/` to GitHub
Pages on every change. Enable it once under Settings → Pages → Source:
GitHub Actions.
