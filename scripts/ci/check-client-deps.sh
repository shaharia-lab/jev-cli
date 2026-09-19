#!/usr/bin/env bash
# jev-client is a standalone library: it must not pull in command-line, terminal or
# configuration-file crates. Those concerns belong to jev-cli. This guards the boundary.
set -euo pipefail

FORBIDDEN=(
  clap clap_builder clap_derive clap_complete clap_mangen
  crossterm console indicatif dialoguer termcolor owo-colors comfy-table tabled
  toml toml_edit config figment directories dirs etcetera
  keyring
)

cd "$(dirname "$0")/../.."
deps=$(cargo tree --locked -p jev-client --edges normal,build --prefix none --format '{p}' | awk '{print $1}' | sort -u)

failed=0
for crate in "${FORBIDDEN[@]}"; do
  if grep -qx -- "$crate" <<<"$deps"; then
    echo "error: jev-client depends on '$crate', which belongs in jev-cli" >&2
    failed=1
  fi
done

if [ "$failed" -ne 0 ]; then
  echo "See 'Architecture' in CLAUDE.md for the crate boundary." >&2
  exit 1
fi
echo "jev-client dependency boundary OK ($(wc -l <<<"$deps") crates checked)"
