#!/usr/bin/env bash
# Builds the sandbox the demo records in: sample files in /tmp/vaqum-demo
# (which is also $HOME during the recording, so the prompt reads `~ ❯`)
# and a clean zsh config, so the recording never picks up your own shell
# theme or plugins. Called by demo/render.sh.
#
#   VAQUM_DEMO_BIN  directory holding the vaqum binary to record
set -euo pipefail

DEMO_DIR="/tmp/vaqum-demo"
ZDOTDIR_DEMO="/tmp/vaqum-demo-zsh"
BIN_DIR="${VAQUM_DEMO_BIN:?set VAQUM_DEMO_BIN to the directory holding the vaqum binary}"

rm -rf "$DEMO_DIR" "$ZDOTDIR_DEMO"
mkdir -p "$DEMO_DIR" "$ZDOTDIR_DEMO"
cd "$DEMO_DIR"

# ---------------------------------------------------------------- shell
cat > "$ZDOTDIR_DEMO/.zshrc" <<EOF
export HOME="$DEMO_DIR"
export PATH="$BIN_DIR:\$PATH"
cd "\$HOME"
PROMPT='%F{#a6adc8}%~%f %F{#b4befe}❯%f '
RPROMPT=''
PROMPT_EOL_MARK=''
unsetopt beep
EOF

# ---------------------------------------------------------------- files

# compress
{
  echo "Q3 Planning"
  echo "==========="
  for i in $(seq 1 400); do
    echo "- action item $i: review budget line $i and confirm the owner"
  done
} > report.txt

# list / --only
mkdir -p project/src project/docs
printf 'fn main() {\n    println!("hello from vaqum");\n}\n' > project/src/main.rs
printf 'pub fn helper() -> u32 {\n    42\n}\n' > project/src/util.rs
printf '# Project\n\nA small sample project.\n' > project/README.md
for i in $(seq 1 60); do echo "Section $i: notes about the design."; done > project/docs/design.md

# pipes
for i in $(seq 1 3000); do
  printf '2026-11-20T10:%02d:%02dZ INFO request id=%d path=/api/items status=200 ms=%d\n' \
    $((i / 60 % 60)) $((i % 60)) "$i" $((i % 90 + 3))
done > server.log

# encrypt / shred
{
  echo "# production credentials"
  for svc in stripe github aws sendgrid twilio postgres redis sentry datadog openai; do
    name="$(printf '%s' "$svc" | tr '[:lower:]' '[:upper:]')"
    echo "${name}_API_KEY=sk_live_$(printf '%s' "$svc" | shasum -a 256 | cut -c1-40)"
    echo "${name}_API_URL=https://api.$svc.example.com/v1"
  done
} > secret.txt

# dedup
mkdir -p photos
cp report.txt photos/beach.jpg
cp report.txt photos/beach-copy.jpg
printf 'one-of-a-kind photo\n' > photos/sunset.jpg

# diff
printf 'Meeting notes\nDiscuss roadmap\nShip v1.0\n' > notes-v1.txt
printf 'Meeting notes\nDiscuss roadmap\nShip v1.1\nFix Windows CI\n' > notes-v2.txt

# search, and the leak check (token text built at runtime, so it never
# appears verbatim in the repository)
mkdir -p app/TODO-cleanup app/config
printf 'DATABASE_URL=postgres://app:hunter2@db.internal/prod\n' > app/.env
printf 'AWS_ACCESS_KEY_ID = "%s"\n' "AKIA$(printf 'X%.0s' 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16)" > app/config/settings.py
printf 'fn main() {\n    // TODO: wire up config\n}\n' > app/main.rs
printf '// TODO: add tests\npub fn helper() {}\n' > app/util.rs

# encrypt for a person: Bob's key, and something to send him
{
  echo "Q4 launch plan"
  for i in $(seq 1 60); do echo "- milestone $i: owner, budget and review date for workstream $i"; done
} > plans.txt
"$BIN_DIR/vaqum" keygen -o bob.key >/dev/null
echo "export BOB=\"$("$BIN_DIR/vaqum" pubkey -i bob.key)\"" >> "$ZDOTDIR_DEMO/.zshrc"

# zip / tar.gz
mkdir -p archive-src/photos
printf 'shared with a teammate\n' > archive-src/notes.txt
printf 'holiday\n' > archive-src/photos/trip.txt
(cd archive-src && zip -qr ../shared.zip .)
rm -rf archive-src
