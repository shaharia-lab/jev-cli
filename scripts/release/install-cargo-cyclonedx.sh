#!/usr/bin/env bash
# Installs a pinned, checksum-verified release binary of cargo-cyclonedx (which writes the release
# SBOM) on a Linux x86_64 CI runner, like scripts/ci/install-audit-tools.sh does for its tools.
#
# To upgrade: bump the version, and take the checksum from the `.sha256` file published beside the
# archive after checking it against `sha256sum` of the download.
set -euo pipefail

VERSION="0.5.9"
SHA256="fb8dbee9f182173e062a64a387b21a0badc6fab8b2abf9294973f012972bf6d8"
TARGET="x86_64-unknown-linux-gnu"
BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"

if [ "$(uname -s)-$(uname -m)" != "Linux-x86_64" ]; then
  echo "error: this installer supports Linux x86_64 only; use 'cargo install --locked cargo-cyclonedx'" >&2
  exit 1
fi

url="https://github.com/CycloneDX/cyclonedx-rust-cargo/releases/download/cargo-cyclonedx-$VERSION/cargo-cyclonedx-$TARGET.tar.xz"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
echo "Installing cargo-cyclonedx from $url"
curl --proto '=https' --proto-redir '=https' --tlsv1.2 --fail --silent --show-error --location --retry 3 "$url" -o "$tmp/archive.tar.xz"
echo "$SHA256  $tmp/archive.tar.xz" | sha256sum --check --quiet
tar -xJf "$tmp/archive.tar.xz" -C "$tmp" --strip-components=1
mkdir -p "$BIN_DIR"
install -m 0755 "$tmp/cargo-cyclonedx" "$BIN_DIR/cargo-cyclonedx"
cargo cyclonedx --version
