#!/usr/bin/env bash
# Tests vaqum in Linux containers before a release.
#
#   ./scripts/container-test.sh            # everything
#   ./scripts/container-test.sh build      # test suite + static binaries
#   ./scripts/container-test.sh smoke      # static binaries on many distros
#   ./scripts/container-test.sh npm        # npm install on several Node versions
#
# Needs Docker (Docker Desktop, colima, Rancher Desktop, ...). On an ARM Mac,
# x86_64 runs under emulation, which is slow and can crash the C compiler;
# ARCHES=arm64 limits the run to native containers.
#
# Stages:
#   build  Runs `cargo test` on Debian (glibc) and Alpine (musl) for arm64 and
#          x86_64, and builds the static musl binaries the release ships.
#   smoke  Runs a feature smoke test with those binaries on old and new
#          distros (including ones with no glibc at all).
#   npm    Packs the npm package plus a linux platform package and installs
#          it in Node 16-22 containers, then runs vaqum through it.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/target/container-test"
STAGE="${1:-all}"
read -r -a ARCH_LIST <<< "${ARCHES:-arm64 amd64}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n 1)"
mkdir -p "$OUT"

pass=()
fail=()
record() { # $1 = label, $2 = exit code
  if [ "$2" -eq 0 ]; then pass+=("$1"); echo "   ✔ $1"; else fail+=("$1"); echo "   ✘ $1 (log: $OUT/logs/$(echo "$1" | tr ' /:' '___').log)"; fi
}
run() { # $1 = label, rest = docker run args
  local label="$1"; shift
  mkdir -p "$OUT/logs"
  local log="$OUT/logs/$(echo "$label" | tr ' /:' '___').log"
  set +e
  docker run --rm "$@" >"$log" 2>&1
  local code=$?
  set -e
  record "$label" "$code"
}

# Copy the source tree into the container without the host's build output.
COPY_SRC='mkdir -p /work && tar -C /src --exclude=./target --exclude=./fuzz/target --exclude=./fuzz/corpus --exclude=./npm/node_modules -cf - . | tar -C /work -xf - && cd /work'

stage_build() {
  echo "==> build: test suite and static binaries"
  for arch in "${ARCH_LIST[@]}"; do
    run "cargo test, Debian glibc, $arch" --platform "linux/$arch" \
      -v "$ROOT:/src:ro" -v "vaqum-cargo-$arch:/usr/local/cargo/registry" \
      rust:1-bookworm sh -c "$COPY_SRC && cargo test --locked"

    run "cargo test + static build, Alpine musl, $arch" --platform "linux/$arch" \
      -v "$ROOT:/src:ro" -v "$OUT:/out" -v "vaqum-cargo-$arch:/usr/local/cargo/registry" \
      rust:1-alpine sh -c "apk add --no-cache build-base >/dev/null && $COPY_SRC \
        && cargo test --locked && cargo build --release --locked \
        && cp target/release/vaqum /out/vaqum-linux-$arch-musl \
        && (ldd target/release/vaqum 2>&1 || true) | tee /out/ldd-$arch.txt"
  done
}

stage_smoke() {
  echo "==> smoke: static binaries on many distros"
  local images_arm64=(alpine:3.20 busybox:1.36-musl debian:bookworm-slim ubuntu:20.04 ubuntu:24.04 fedora:41 amazonlinux:2)
  local images_amd64=(alpine:3.20 ubuntu:18.04 debian:bookworm-slim centos:7 fedora:41)
  for arch in "${ARCH_LIST[@]}"; do
    local bin="$OUT/vaqum-linux-$arch-musl"
    if [ ! -x "$bin" ]; then
      echo "   (skipping $arch: run the build stage first)"
      continue
    fi
    local images
    if [ "$arch" = arm64 ]; then images=("${images_arm64[@]}"); else images=("${images_amd64[@]}"); fi
    for image in "${images[@]}"; do
      run "smoke $image $arch" --platform "linux/$arch" \
        -v "$bin:/opt/vaqum:ro" -v "$ROOT/scripts/smoke.sh:/smoke.sh:ro" \
        "$image" sh /smoke.sh /opt/vaqum
    done
  done
}

stage_npm() {
  echo "==> npm: install the packed package on several Node versions"
  local arch=arm64
  case "$(uname -m)" in x86_64 | amd64) arch=amd64 ;; esac
  local bin="$OUT/vaqum-linux-$arch-musl"
  if [ ! -x "$bin" ]; then
    echo "   (skipping: run the build stage first)"
    return
  fi
  local npm_arch=arm64
  [ "$arch" = amd64 ] && npm_arch=x64

  # A linux platform package built from the static binary, like release CI.
  local pkgdir="$OUT/npm-pkgs"
  rm -rf "$pkgdir" && mkdir -p "$pkgdir/platform/bin"
  cp "$bin" "$pkgdir/platform/bin/vaqum"
  cat > "$pkgdir/platform/package.json" <<EOF
{ "name": "@marcdomain/vaqum-linux-$npm_arch", "version": "$VERSION", "os": ["linux"], "cpu": ["$npm_arch"], "files": ["bin/"], "license": "MIT" }
EOF
  (cd "$pkgdir/platform" && npm pack --silent --pack-destination "$pkgdir" >/dev/null)
  (cd "$ROOT/npm" && npm pack --silent --ignore-scripts --pack-destination "$pkgdir" >/dev/null)

  local install='set -e; mkdir /app && cd /app && npm init -y >/dev/null \
    && npm install --no-audit --no-fund /pkgs/*.tgz 2>&1 | tail -n 3 \
    && test -d node_modules/@marcdomain/vaqum-linux-'"$npm_arch"' \
    && ./node_modules/.bin/vaqum --version | grep -q "vaqum '"$VERSION"'" \
    && echo hello | ./node_modules/.bin/vaqum compress - 2>/dev/null | ./node_modules/.bin/vaqum decompress - -o - 2>/dev/null | grep -q hello \
    && echo NPM OK'
  for image in node:16-alpine node:18-bullseye-slim node:20-bookworm-slim node:22-alpine; do
    run "npm install on $image" --platform "linux/$arch" -v "$pkgdir:/pkgs:ro" "$image" sh -c "$install"
  done
  # The npm package's own tests (they need Node 18+).
  for image in node:18-bullseye-slim node:22-alpine; do
    run "npm test on $image" --platform "linux/$arch" -v "$ROOT/npm:/src:ro" "$image" \
      sh -c 'mkdir /w && cp -r /src/. /w && cd /w && npm test'
  done
}

case "$STAGE" in
  build) stage_build ;;
  smoke) stage_smoke ;;
  npm) stage_npm ;;
  all) stage_build; stage_smoke; stage_npm ;;
  *) echo "usage: $0 [all|build|smoke|npm]" >&2; exit 2 ;;
esac

echo
echo "==> ${#pass[@]} passed, ${#fail[@]} failed"
for f in "${fail[@]}"; do echo "   ✘ $f"; done
[ "${#fail[@]}" -eq 0 ]
