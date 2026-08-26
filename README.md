# vaqum

[![CI](https://github.com/marcdomain/vaqum/actions/workflows/ci.yml/badge.svg)](https://github.com/marcdomain/vaqum/actions/workflows/ci.yml)

Losslessly compress files 5x faster and 12% smaller output than zip/gzip at defaults (optionally encrypted), decompress them, diff (displayed on editor or HTML report), search and find duplicates, and securely shred them — from the command line.

![vaqum demo](demo.gif)

## Install

```sh
brew tap marcdomain/vaqum && brew install vaqum   # macOS / Linux
npm i -g @marcdomain/vaqum                        # macOS / Linux / Windows
curl -fsSL https://raw.githubusercontent.com/marcdomain/vaqum/main/install.sh | sh   # macOS / Linux, no deps
irm https://raw.githubusercontent.com/marcdomain/vaqum/main/install.ps1 | iex        # Windows, no deps
cargo install --path .                            # from source
```

The Homebrew tap + install can also be done in one line:
`brew install marcdomain/vaqum/vaqum` (`user/tap/formula`).

## Usage

```sh
vaqum compress <path>... [-o out] [-l 1-22] [--max] [-t threads] [-r] [--dedup] [--dry-run] [-v] [-e] [--key-file <f>]
vaqum decompress <path.vaqum> [-o dir] [-v] [--verify] [--key-file <f>] [--max-ratio <n>] [--force]
vaqum diff <a> <b> [--html report.html] [--open] [-e|--editor] [-v]   # files, dirs, or .vaqum archives, any mix
vaqum dedupe <dir> [--link] [--dry-run] [-t threads] [-v]             # find (and optionally hardlink) duplicate files
vaqum shred <path> [-r] [-p passes] [-y] [--dry-run]
vaqum info <path.vaqum>
```

Run `vaqum <command> --help` for full flag documentation.

`compress` accepts more than one file/directory at once, bundling them into a single archive; `-o` is then required to name it, e.g. `vaqum compress a.txt notes/ -r -o bundle.vaqum`.

`diff` has three stackable output modes:

- *(default)* — unified diff printed to the terminal
- `--html <file>` — self-contained, offline, side-by-side HTML report
- `--open` — same HTML report, written to a temp file and opened in your default browser (skip `--html` to use this alone)
- `-e, --editor` — hands off to `code --diff a b` (VS Code's live, editable split view); override the editor with `$VAQUM_DIFF_EDITOR`. For directories, opens one tab per modified text file. A `.vaqum` side is decompressed to a scratch copy first — edits there don't save back into the archive.

`compress -e` (or `--key-file <f>`) encrypts the output with ChaCha20-Poly1305, keyed by an Argon2id-derived password (or the keyfile's bytes) — the key itself is never stored, only the salt. `decompress` auto-detects encryption and prompts for the password (or takes `--key-file`).

`decompress` also refuses archives that claim an implausible expansion ratio (>1000:1 by default) or would exceed available disk space, to guard against decompression bombs; tune with `--max-ratio` or bypass with `--force`.

## Development

```sh
cargo build
cargo test                                    # spawns the real binary (tests/cli.rs)
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

CI runs the same checks on Linux, macOS, and Windows for every push/PR.
Tagged releases (`vX.Y.Z`) trigger `.github/workflows/release.yml`, which
cross-builds binaries, publishes a GitHub Release, and pushes to npm and
the Homebrew tap — see `packaging/homebrew/README.md` for tap setup.

Re-record the demo above with [VHS](https://github.com/charmbracelet/vhs)
after `cargo install --path .`: `vhs demo.tape` (uses `demo-setup.sh` for
its sample files).

## License

MIT
