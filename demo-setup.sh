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
