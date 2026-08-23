# vaqum

Losslessly compress files faster and smaller than zip/gzip, decompress them, diff (displayed on editor or HTML report), search and find duplicates, and securely shred them — from the command line.

## Install

```bash
npm i -g @marcdomain/vaqum
```

![vaqum demo](https://raw.githubusercontent.com/marcdomain/vaqum/main/demo.gif)

This package is a thin wrapper: `npm install` fetches the real native
`vaqum` binary for your platform from the matching [GitHub
Release](https://github.com/marcdomain/vaqum/releases), verifies its
checksum, and installs it as the `vaqum` command.

## Usage

```sh
vaqum compress <path> [-o out] [-l 1-22] [--max] [-r] [--dedup] [-v]
vaqum decompress <path.vaqum> [-o dir] [--verify]
vaqum diff <a> <b> [--html report.html] [--open] [--editor]
vaqum dedupe <dir> [--link]
vaqum search <pattern> [path] [-i] [-E]
vaqum shred <path> [-r] [-p passes] [-y]
vaqum info <path.vaqum>
```

Run `vaqum <command> --help` for full flag documentation.

Full docs, source, and issues: <https://github.com/marcdomain/vaqum>
