# vaqum — demo tape
# Run with: vhs demo.tape

Output demo.gif

# ── Terminal settings ──────────────────────────────────────────────────────────
Set FontSize 14
Set Width 1000
Set Height 640
Set Theme "Catppuccin Mocha"
Set Shell "zsh"
Set Padding 24
Set TypingSpeed 40ms
Set PlaybackSpeed 1

# ── Setup: sample files, hidden from the recording ─────────────────────────────
# (built by demo-setup.sh — kept out of the tape so it isn't fighting VHS's
# Type-string quoting rules for a multi-line loop)
# CARGO_BIN is captured before HOME is overridden below, and put ahead of
# PATH, so the recording runs the freshly built dev binary rather than
# whatever's on PATH already (e.g. a globally npm-installed release build).
Hide
Type "export CARGO_BIN=$HOME/.cargo/bin && bash demo-setup.sh && cd /tmp/vaqum-demo && export HOME=/tmp/vaqum-demo && export PATH=$CARGO_BIN:$PATH"
Enter
Sleep 500ms
Type "clear"
Enter
Sleep 200ms
Show

# ══════════════════════════════════════════════════════════════════════════════
# 1. HELP
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum --help"
Enter
Sleep 3s

# ══════════════════════════════════════════════════════════════════════════════
# 2. COMPRESS a single file, then INFO and DECOMPRESS --verify
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum info report.txt"
Enter
Sleep 2500ms

Type "vaqum compress report.txt -v"
Enter
Sleep 2500ms

Type "vaqum info report.txt.vaqum"
Enter
Sleep 2500ms

Type "vaqum decompress report.txt.vaqum -o restored --verify"
Enter
Sleep 2500ms

# ══════════════════════════════════════════════════════════════════════════════
# 3. COMPRESS --max — LZMA/xz mode for a smaller, slower result
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum compress report.txt --max -o report-max.vaqum"
Enter
Sleep 2500ms

Type "vaqum info report-max.vaqum"
Enter
Sleep 2500ms

# ══════════════════════════════════════════════════════════════════════════════
# 4. COMPRESS multiple paths at once into one bundle
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum compress report.txt notes-v1.txt notes-v2.txt -o bundle.vaqum"
Enter
Sleep 2500ms

Type "vaqum info bundle.vaqum"
Enter
Sleep 2500ms

# ══════════════════════════════════════════════════════════════════════════════
# 5. COMPRESS a directory with --dedup, then DEDUPE and --link
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum info photos"
Enter
Sleep 2500ms

Type "vaqum compress photos -r --dedup -v"
Enter
Sleep 3s

Type "vaqum dedupe photos -v"
Enter
Sleep 2500ms

Type "vaqum dedupe photos --link"
Enter
Sleep 2000ms

# ══════════════════════════════════════════════════════════════════════════════
# 6. COMPRESS --exclude — skip a directory (and a file pattern) by name
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum compress webapp -r --exclude node_modules/ -v -o webapp.vaqum"
Enter
Sleep 3s

# ══════════════════════════════════════════════════════════════════════════════
# 7. CONFIG — persisted defaults/profiles for compress
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum config show"
Enter
Sleep 2500ms

Type "vaqum compress report.txt --profile release -o report-release.vaqum"
Enter
Sleep 2500ms

Type "vaqum info report-release.vaqum"
Enter
Sleep 2500ms

# ══════════════════════════════════════════════════════════════════════════════
# 8. DECOMPRESS/INFO — reading foreign zip and tar.gz archives
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum info shared.zip"
Enter
Sleep 2500ms

Type "vaqum decompress shared.tar.gz -o shared-restored"
Enter
Sleep 2500ms

# ══════════════════════════════════════════════════════════════════════════════
# 9. DIFF — two files, unified diff in the terminal
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum diff notes-v1.txt notes-v2.txt"
Enter
Sleep 3s

# ══════════════════════════════════════════════════════════════════════════════
# 10. DIFF — two directories (added/removed/modified summary)
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum diff project-backup project -v"
Enter
Sleep 3500ms

# ══════════════════════════════════════════════════════════════════════════════
# 11. DIFF --html — self-contained side-by-side report
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum diff notes-v1.txt notes-v2.txt --html notes.diff.html"
Enter
Sleep 2500ms

# ══════════════════════════════════════════════════════════════════════════════
# 12. SEARCH — by name and content together, then a regex/content-only pass
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum search TODO app"
Enter
Sleep 2500ms

Type "vaqum search 'TODO: \\w+' app -E --content-only"
Enter
Sleep 2500ms

# ══════════════════════════════════════════════════════════════════════════════
# 13. COMPRESS -e — password-encrypted archive, then DECOMPRESS with it
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum compress secret.txt -e -o secret.vaqum"
Enter
Sleep 500ms
Type "hunter2"
Enter
Sleep 300ms
Type "hunter2"
Enter
Sleep 2500ms

Type "vaqum decompress secret.vaqum -o restored-secret.txt"
Enter
Sleep 500ms
Type "hunter2"
Enter
Sleep 2000ms

# ══════════════════════════════════════════════════════════════════════════════
# 14. COMPLETIONS --install — write a shell completion script into place
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum completions zsh --install"
Enter
Sleep 2500ms

# ══════════════════════════════════════════════════════════════════════════════
# 15. SHRED — safety prompt, then confirmed destruction
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum shred secret.txt"
Enter
Sleep 1500ms
Type "secret.txt"
Enter
Sleep 2500ms

# ══════════════════════════════════════════════════════════════════════════════
# END
# ══════════════════════════════════════════════════════════════════════════════
Sleep 2s
