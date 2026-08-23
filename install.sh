#!/usr/bin/env sh
# vaqum installer (macOS / Linux) — downloads a release binary from GitHub
# and verifies its checksum. No cargo, no package manager required.
#
#   curl -fsSL https://raw.githubusercontent.com/marcdomain/vaqum/main/install.sh | sh
#
# Pin a version or install location:
#   curl -fsSL .../install.sh | VAQUM_VERSION=0.2.0 VAQUM_INSTALL_DIR=/usr/local/bin sh

set -eu

REPO="marcdomain/vaqum"
INSTALL_DIR="${VAQUM_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf 'vaqum: %s\n' "$1"; }
die() {
  printf 'vaqum: error: %s\n' "$1" >&2
  exit 1
}

sha256_check() {
  # $1 = file, $2 = sidecar (both in cwd, sidecar records the bare filename)
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "$2" >/dev/null
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c "$2" >/dev/null
  else
    die "neither sha256sum nor shasum found; cannot verify download integrity"
  fi
}

detect_target() {
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Darwin)
      case "$arch" in
        arm64) echo "aarch64-apple-darwin" ;;
        x86_64) echo "x86_64-apple-darwin" ;;
        *) die "unsupported macOS architecture: $arch" ;;
      esac
      ;;
    Linux)
      case "$arch" in
        x86_64) echo "x86_64-unknown-linux-gnu" ;;
        *) die "unsupported Linux architecture: $arch (try 'cargo install vaqum' instead)" ;;
      esac
      ;;
    *) die "unsupported OS: $os (on Windows, use install.ps1; otherwise try 'cargo install vaqum')" ;;
  esac
}

version="${VAQUM_VERSION:-}"
if [ -z "$version" ]; then
  say "resolving latest release..."
  version="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep -m1 '"tag_name"' | sed -E 's/.*"v([^"]+)".*/\1/')"
  [ -n "$version" ] || die "could not resolve the latest release version"
fi

target="$(detect_target)"
asset="vaqum-${version}-${target}.tar.gz"
url="https://github.com/${REPO}/releases/download/v${version}/${asset}"

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

say "downloading ${asset} (v${version}, ${target})..."
curl -fsSL "$url" -o "$work_dir/$asset" || die "failed to download $url"
curl -fsSL "${url}.sha256" -o "$work_dir/$asset.sha256" || die "failed to download checksum"

say "verifying checksum..."
(cd "$work_dir" && sha256_check "$asset" "$asset.sha256") || die "checksum verification failed"

say "extracting..."
tar -xzf "$work_dir/$asset" -C "$work_dir"

mkdir -p "$INSTALL_DIR"
mv "$work_dir/vaqum" "$INSTALL_DIR/vaqum"
chmod +x "$INSTALL_DIR/vaqum"

say "installed to $INSTALL_DIR/vaqum"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    say "note: $INSTALL_DIR is not on your PATH. Add this to your shell profile:"
    say "  export PATH=\"$INSTALL_DIR:\$PATH\""
    ;;
esac

"$INSTALL_DIR/vaqum" --version
