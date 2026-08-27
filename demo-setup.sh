#!/usr/bin/env bash
# Builds the sample files used by demo.tape. Not part of vaqum itself —
# just recording-time scaffolding, kept as a real script (rather than
# inlined into the tape) so it isn't fighting VHS's Type-string quoting.
set -euo pipefail

DEMO_DIR="/tmp/vaqum-demo"
rm -rf "$DEMO_DIR"
mkdir -p "$DEMO_DIR/photos"
cd "$DEMO_DIR"

{
  echo "Q3 Planning"
  echo "============"
  for i in $(seq 1 200); do
    echo "- action item $i: review budget line $i and confirm the owner"
  done
} > report.txt

cp report.txt photos/img1.txt
cp report.txt photos/img2.txt
printf 'one-of-a-kind photo notes\n' > photos/img3.txt

printf 'Meeting notes\n- Discuss roadmap\n- Ship v1.0\n' > notes-v1.txt
printf 'Meeting notes\n- Discuss roadmap\n- Ship v1.0\n- Fix Windows CI\n' > notes-v2.txt

printf 'super secret api key: 12345\n' > secret.txt

# For `diff` on directories: a "backup" vs the current "project" tree —
# one file edited, one file added.
mkdir -p project-backup/src project/src
printf 'fn main() {\n    println!("v1");\n}\n' > project-backup/src/main.rs
printf '# My Project\n' > project-backup/README.md

printf 'fn main() {\n    println!("v2");\n}\n' > project/src/main.rs
printf '# My Project\n' > project/README.md
printf 'pub fn helper() {}\n' > project/src/util.rs

# For `compress --exclude`: its own tree (kept separate from
# project/project-backup above so it doesn't clutter the diff demo).
mkdir -p webapp/src webapp/node_modules/left-pad
printf 'fn main() {}\n' > webapp/src/main.rs
printf '# webapp\n' > webapp/README.md
printf 'module.exports = () => {};\n' > webapp/node_modules/left-pad/index.js

# For `search`: a couple of files with TODO markers to find by content,
# and a TODO-named directory to find by name.
mkdir -p app/TODO-cleanup
printf 'fn main() {\n    // TODO: wire up config\n}\n' > app/main.rs
printf '// TODO: add tests\npub fn helper() {}\n' > app/util.rs
