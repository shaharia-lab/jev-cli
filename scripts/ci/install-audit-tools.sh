#!/usr/bin/env bash
# Installs pinned, checksum-verified release binaries of cargo-deny and cargo-audit on a Linux
# x86_64 CI runner. Prebuilt binaries keep the job fast; the pinned SHA-256 means a replaced
# release asset fails the build instead of running.
#
# To upgrade a tool: bump its version, download the new archive, and update the checksum with the
# output of `sha256sum`. For cargo-deny, compare it with the `.sha256` file published beside the
# archive.
set -euo pipefail

CARGO_DENY_VERSION="0.20.2"
CARGO_DENY_SHA256="9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f"

CARGO_AUDIT_VERSION="0.22.2"
CARGO_AUDIT_SHA256="7fb9497f8594b389e5fce5ef9b92db08432996895b2e0c5a0167a69ed445c428"

TARGET="x86_64-unknown-linux-musl"
BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"

if [ "$(uname -s)-$(uname -m)" != "Linux-x86_64" ]; then
  echo "error: this installer supports Linux x86_64 only; use 'cargo install --locked cargo-deny cargo-audit'" >&2
  exit 1
fi

install_tool() {
  local name=$1 url=$2 sha256=$3
  local tmp
  tmp=$(mktemp -d)

  echo "Installing $name from $url"
  curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location --retry 3 "$url" -o "$tmp/archive.tar.gz"
  echo "$sha256  $tmp/archive.tar.gz" | sha256sum --check --quiet
  tar -xzf "$tmp/archive.tar.gz" -C "$tmp" --strip-components=1
  install -m 0755 "$tmp/$name" "$BIN_DIR/$name"
  rm -rf "$tmp"
}

mkdir -p "$BIN_DIR"

install_tool cargo-deny \
  "https://github.com/EmbarkStudios/cargo-deny/releases/download/${CARGO_DENY_VERSION}/cargo-deny-${CARGO_DENY_VERSION}-${TARGET}.tar.gz" \
  "$CARGO_DENY_SHA256"

install_tool cargo-audit \
  "https://github.com/rustsec/rustsec/releases/download/cargo-audit%2Fv${CARGO_AUDIT_VERSION}/cargo-audit-${TARGET}-v${CARGO_AUDIT_VERSION}.tgz" \
  "$CARGO_AUDIT_SHA256"

cargo deny --version
cargo audit --version
