# vaqum

[![CI](https://github.com/marcdomain/vaqum/actions/workflows/ci.yml/badge.svg)](https://github.com/marcdomain/vaqum/actions/workflows/ci.yml)

Losslessly compress/decompress files, diff them (with an HTML report),
find duplicates, and securely shred them — from the command line.
See [`vaqum.md`](./vaqum.md) for the design brief.

## Install

```sh
brew tap marcdomain/vaqum && brew install vaqum   # macOS / Linux
npm install -g vaqum                              # macOS / Linux / Windows
curl -fsSL https://raw.githubusercontent.com/marcdomain/vaqum/main/install.sh | sh   # macOS / Linux, no deps
irm https://raw.githubusercontent.com/marcdomain/vaqum/main/install.ps1 | iex        # Windows, no deps
cargo install --path .                            # from source
```

The Homebrew tap + install can also be done in one line:
`brew install marcdomain/vaqum/vaqum` (`user/tap/formula`).

## Usage

```sh
vaqum compress <path> [-o out] [-l 1-22] [--max] [-t threads] [-r] [--dedup] [--dry-run] [-v]
vaqum decompress <path.vaqum> [-o dir] [-v] [--verify]
vaqum diff <a> <b> [--html report.html] [--open] [-v]   # files, dirs, or .vaqum archives, any mix
vaqum dedupe <dir> [--link] [--dry-run] [-v]             # find (and optionally hardlink) duplicate files
vaqum shred <path> [-r] [-p passes] [-y] [--dry-run]
vaqum info <path.vaqum>
```

Run `vaqum <command> --help` for full flag documentation.

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

## License

MIT
