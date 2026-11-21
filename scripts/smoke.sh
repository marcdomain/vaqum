#!/bin/sh
# Feature smoke test for a vaqum binary, POSIX sh only (runs on BusyBox).
# Used by scripts/container-test.sh:  sh smoke.sh /path/to/vaqum
set -eu

V="$1"
work="$(mktemp -d)"
cd "$work"
step() { echo "-- $1"; }
# Byte-for-byte comparison; minimal images (e.g. Fedora) may lack cmp.
same() {
  if command -v cmp >/dev/null 2>&1; then cmp -s "$1" "$2"; else
    [ "$(sha256sum < "$1")" = "$(sha256sum < "$2")" ]; fi
}

step "version"
"$V" --version

step "compress a directory"
mkdir -p proj/src proj/docs
echo 'fn main() {}' > proj/src/main.rs
echo '# docs' > proj/docs/intro.md
i=0
while [ "$i" -lt 3000 ]; do echo "log line $i: the quick brown fox"; i=$((i + 1)); done > proj/big.txt
"$V" compress proj -r -o p.vqm >/dev/null

step "list (text and json)"
"$V" list p.vqm | grep -q 'proj/src/main.rs'
"$V" list --json p.vqm | grep -q '"files": 3'

step "--only with a wildcard, verified"
"$V" decompress p.vqm -o out --only '*.rs' --verify >/dev/null
test -f out/proj/src/main.rs
test ! -e out/proj/big.txt

step "pipes: stdin -> stdout round trip"
"$V" compress - < proj/big.txt 2>/dev/null > big.vqm
"$V" decompress big.vqm -o - 2>/dev/null > big.out
same big.out proj/big.txt

step "encryption with a key file"
printf 'k3y-material' > key
"$V" compress proj/big.txt --key-file key -o enc.vqm >/dev/null
mkdir dec
"$V" decompress enc.vqm --key-file key -o dec --verify >/dev/null
same dec/big.txt proj/big.txt
printf 'wrong' > badkey
if "$V" decompress enc.vqm --key-file badkey -o dec2 >/dev/null 2>&1; then
  echo "wrong key was accepted"; exit 1
fi

step "dedup round trip"
cp proj/big.txt proj/copy.txt
"$V" compress proj -r --dedup -o d.vqm >/dev/null
"$V" decompress d.vqm -o dd --verify >/dev/null
same dd/proj/copy.txt proj/big.txt

step "hostile archive is refused"
# header: magic, v2, single file, zstd, no flags, size 6, zero checksum/salt/nonce, name "../escaped"
{
  printf 'VAQM\002\000\000\000\006\000\000\000\000\000\000\000'
  head -c 55 /dev/zero
  printf '\012\000../escaped'
} > evil.vqm
mkdir evil-out
if "$V" decompress evil.vqm -o evil-out >/dev/null 2>&1; then
  echo "hostile archive was accepted"; exit 1
fi
test ! -e escaped

step "personal keys: encrypt for a recipient, sign, verify"
export XDG_CONFIG_HOME="$work/cfg"
"$V" keygen >/dev/null
me="$("$V" pubkey)"
"$V" keygen -o other.key >/dev/null
"$V" compress proj/big.txt --to "$me" --sign -o to.vqm >/dev/null
"$V" verify to.vqm --from "$me" >/dev/null
mkdir to-out
"$V" decompress to.vqm --from "$me" -o to-out --verify >/dev/null 2>&1
same to-out/big.txt proj/big.txt
if "$V" list to.vqm -i other.key >/dev/null 2>&1; then
  echo "a non-recipient opened the archive"; exit 1
fi

step "leak check"
mkdir leaky
echo "DATABASE_URL=postgres://u:p@h/db" > leaky/.env
"$V" compress leaky -r -o leaky.vqm 2>leak.txt >/dev/null
grep -q "Possible secrets" leak.txt

step "shred"
echo secret > s.txt
"$V" shred s.txt -y >/dev/null
test ! -e s.txt

echo "SMOKE OK"
