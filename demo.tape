# vaqum — demo tape
# Run with: vhs demo.tape

Output demo.gif

# ── Terminal settings ──────────────────────────────────────────────────────────
Set FontSize 14
Set Width 1000
Set Height 580
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
# 2. COMPRESS a single file
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum compress report.txt -v"
Enter
Sleep 2500ms

Type "vaqum info report.txt.vaqum"
Enter
Sleep 2500ms

# ══════════════════════════════════════════════════════════════════════════════
# 3. DECOMPRESS with checksum verification
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum decompress report.txt.vaqum -o restored --verify"
Enter
Sleep 2500ms

# ══════════════════════════════════════════════════════════════════════════════
# 4. COMPRESS a directory, with --dedup (photos/img1.txt and img2.txt are identical)
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum compress photos -r --dedup -v"
Enter
Sleep 3s

# ══════════════════════════════════════════════════════════════════════════════
# 5. DEDUPE — find duplicates without compressing
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum dedupe photos -v"
Enter
Sleep 2500ms

# ══════════════════════════════════════════════════════════════════════════════
# 6. DIFF — two files, unified diff in the terminal
# ══════════════════════════════════════════════════════════════════════════════
Type "vaqum diff notes-v1.txt notes-v2.txt"
Enter
Sleep 3s

# ══════════════════════════════════════════════════════════════════════════════
# 7. SHRED — safety prompt, then confirmed destruction
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
