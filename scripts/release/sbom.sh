#!/usr/bin/env bash
# Writes the CycloneDX SBOM of the `jev` binary to <out-dir>/jev-<version>.cdx.json:
#
#   scripts/release/sbom.sh <version> <out-dir>
#
# It lists every crate that can be compiled into `jev` on any of the release targets, with its
# version, licence and checksum from Cargo.lock (jev-client included). Needs cargo-cyclonedx
# (scripts/release/install-cargo-cyclonedx.sh).
set -euo pipefail

if [ $# -ne 2 ]; then
  echo "usage: $0 <version> <out-dir>" >&2
  exit 2
fi
version=$1 out=$2
root=$(cd "$(dirname "$0")/../.." && pwd)

# cargo-cyclonedx writes one file beside the Cargo.toml of every workspace member.
name="jev-sbom-$$"
trap 'rm -f "$root"/crates/*/"$name".json' EXIT
cargo cyclonedx --manifest-path "$root/crates/jev-cli/Cargo.toml" --format json --spec-version 1.5 \
  --target all --override-filename "$name" --quiet
mkdir -p "$out"
mv "$root/crates/jev-cli/$name.json" "$out/jev-$version.cdx.json"

component=$(jq -r '.metadata.component | "\(.name) \(.version)"' "$out/jev-$version.cdx.json")
if [ "$component" != "jev-cli $version" ]; then
  echo "error: the SBOM describes '$component', not jev-cli $version" >&2
  rm -f "$out/jev-$version.cdx.json"
  exit 1
fi
echo "$out/jev-$version.cdx.json"
