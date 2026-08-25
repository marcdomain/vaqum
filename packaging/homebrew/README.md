# vaqum Homebrew tap

Homebrew tap for [vaqum](https://github.com/marcdomain/vaqum) — Losslessly compress files faster and smaller than zip/gzip (optionally encrypted), decompress them, diff (displayed on editor or HTML report), search and find duplicates, and securely shred them — from the command line.

![vaqum demo](https://raw.githubusercontent.com/marcdomain/vaqum/main/demo.gif)

## Install

```sh
brew tap marcdomain/vaqum
brew install vaqum
```

Or in one line: `brew install marcdomain/vaqum/vaqum`.

## Usage

```sh
vaqum compress <path> [-o out] [-l 1-22] [--max] [-r] [--dedup] [-v] [-e] [--key-file <f>]
vaqum decompress <path.vaqum> [-o dir] [--verify] [--key-file <f>] [--max-ratio <n>] [--force]
vaqum diff <a> <b> [--html report.html] [--open] [--editor]
vaqum dedupe <dir> [--link]
vaqum search <pattern> [path] [-i] [-E]
vaqum shred <path> [-r] [-p passes] [-y]
vaqum info <path.vaqum>
```

Run `vaqum <command> --help` for full flag documentation.

Full docs, source, and issues: <https://github.com/marcdomain/vaqum>
