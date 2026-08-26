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
Hide
Type "bash demo-setup.sh && cd /tmp/vaqum-demo"
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
# 6. DIFF — two files, unified diff in the terminal
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum diff notes-v1.txt notes-v2.txt"
Enter
Sleep 3s

# ══════════════════════════════════════════════════════════════════════════════
# 7. DIFF — two directories (added/removed/modified summary)
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum diff project-backup project -v"
Enter
Sleep 3500ms

# ══════════════════════════════════════════════════════════════════════════════
# 8. DIFF --html — self-contained side-by-side report
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum diff notes-v1.txt notes-v2.txt --html notes.diff.html"
Enter
Sleep 2500ms

# ══════════════════════════════════════════════════════════════════════════════
# 9. SEARCH — by name and content together, then a regex/content-only pass
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum search TODO app"
Enter
Sleep 2500ms

Type "vaqum search 'TODO: \\w+' app -E --content-only"
Enter
Sleep 2500ms

# ══════════════════════════════════════════════════════════════════════════════
# 10. COMPRESS -e — password-encrypted archive, then DECOMPRESS with it
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
# 11. SHRED — safety prompt, then confirmed destruction
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
