# vaqum

Losslessly compress files 5x faster and 12% smaller output than zip/gzip at defaults (optionally encrypted), decompress them, diff (displayed on editor or HTML report), search and find duplicates, and securely shred them — from the command line.

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
vaqum compress <path>... [-o out] [-l 1-22] [--max] [-r] [--dedup] [--exclude <pattern>] [-v] [-e] [--key-file <f>]
vaqum decompress <path.vaqum> [-o dir] [--verify] [--key-file <f>] [--max-ratio <n>] [--force]
vaqum diff <a> <b> [--html report.html] [--open] [--editor]
vaqum dedupe <dir> [--link]
vaqum search <pattern> [path] [-i] [-E]
vaqum shred <path> [-r] [-p passes] [-y]
vaqum info <path>
vaqum completions <bash|zsh|fish|powershell|elvish> [--install]
```

`compress` accepts multiple files/directories, bundled into one archive (`-o` required). `--exclude <pattern>` (repeatable, end with `/` for a directory) skips matching paths — also read from a `.vaqumignore` file at each directory's root.

`info` also works on a plain file or directory (not just `.vaqum` archives), showing size, checksum, and timestamps.

`completions --install` writes the completion script to that shell's standard user directory (auto-detected from `$SHELL` if omitted) instead of printing it.

Run `vaqum <command> --help` for full flag documentation.

Full docs, source, and issues: <https://github.com/marcdomain/vaqum>
