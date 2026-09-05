# vaqum

[![CI](https://github.com/marcdomain/vaqum/actions/workflows/ci.yml/badge.svg)](https://github.com/marcdomain/vaqum/actions/workflows/ci.yml)

Losslessly compress files 5x faster and 12% smaller output than zip/gzip at defaults (optionally encrypted), decompress them, diff (displayed on editor or HTML report), search and find duplicates, and securely shred them — from the command line.

![vaqum demo](demo.gif)

## Install

```sh
npm i -g @marcdomain/vaqum                        # macOS / Linux / Windows
curl -fsSL https://raw.githubusercontent.com/marcdomain/vaqum/main/install.sh | sh   # macOS / Linux, no deps
irm https://raw.githubusercontent.com/marcdomain/vaqum/main/install.ps1 | iex        # Windows, no deps
cargo install --path .                            # from source
```

## Usage

```sh
vaqum compress <path>... [-o out] [-l 1-22] [--max] [-t threads] [-r] [--dedup] [--exclude <pattern>] [--profile <name>] [--dry-run] [-v] [-e] [--key-file <f>] [-q]
vaqum decompress <path.vaqum|.zip|.tar.gz> [-o dir] [-v] [--verify] [--key-file <f>] [--max-ratio <n>] [--force] [-q]
vaqum diff <a> <b> [--html report.html] [--open] [-e|--editor] [-v]   # files, dirs, or .vaqum archives, any mix
vaqum dedupe <dir> [--link] [--dry-run] [-t threads] [-v] [-q]        # find (and optionally hardlink) duplicate files
vaqum shred <path> [-r] [-p passes] [-y] [--dry-run] [-q]
vaqum info <path>
vaqum completions <bash|zsh|fish|powershell|elvish> [--install]
vaqum config <show|path|init>
```

Run `vaqum <command> --help` for full flag documentation.

`config` manages `compress`'s persisted defaults at `~/.config/vaqum/config.toml` (`%APPDATA%\vaqum\config.toml` on Windows): `config init` writes a starter file, `config show` prints what's currently resolved, `config path` prints the file's location. A `[defaults]` block applies to every compress; `[compress]` overrides it for compress specifically; a named `[profile.<name>]` overrides both when selected with `compress --profile <name>`. Command-line flags always win over all three.

`info` inspects a `.vaqum` archive (detected by content, not extension) and shows its compression stats; given a plain file or directory instead, it shows size, checksum (files only), and created/modified timestamps.

`decompress` and `info` also read `.zip` and `.tar.gz`/`.tgz` archives (detected by content), as a convenience so you don't need another tool installed just to open something sent to you. This is one-way: vaqum's own format stays the only supported *output* — there's no `compress --zip` or similar.

`completions` prints a shell completion script to stdout; `--install` writes it straight to that shell's standard user completions directory instead (auto-detected from `$SHELL` if you omit the shell name) and prints any one-line rc-file addition still needed to load it — it never edits your shell config itself.

`compress` accepts more than one file/directory at once, bundling them into a single archive; `-o` is then required to name it, e.g. `vaqum compress a.txt notes/ -r -o bundle.vaqum`.

`--exclude <pattern>` (repeatable) skips matching paths under a compressed directory — end it with `/` to exclude a directory (and everything under it), omit it to exclude a file; a name with no `/` matches at any depth. Also read automatically from a `.vaqumignore` file (same syntax, one pattern per line) at each directory's root.

`diff` has three stackable output modes:

- *(default)* — unified diff printed to the terminal
- `--html <file>` — self-contained, offline, side-by-side HTML report
- `--open` — same HTML report, written to a temp file and opened in your default browser (skip `--html` to use this alone)
- `-e, --editor` — hands off to `code --diff a b` (VS Code's live, editable split view); override the editor with `$VAQUM_DIFF_EDITOR`. For directories, opens one tab per modified text file. A `.vaqum` side is decompressed to a scratch copy first — edits there don't save back into the archive.

`compress -e` (or `--key-file <f>`) encrypts the output with ChaCha20-Poly1305, keyed by an Argon2id-derived password (or the keyfile's bytes) — the key itself is never stored, only the salt. `decompress` auto-detects encryption and prompts for the password (or takes `--key-file`).

`decompress` also refuses archives that claim an implausible expansion ratio (>1000:1 by default) or would exceed available disk space, to guard against decompression bombs; tune with `--max-ratio` or bypass with `--force`.

A progress bar shows automatically on `compress`, `decompress`, `dedupe`, and `shred` whenever output is an interactive terminal; it's auto-suppressed when piped/redirected, or explicitly with `-q`/`--quiet`.

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
