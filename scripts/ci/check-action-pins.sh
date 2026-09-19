#!/usr/bin/env bash
# Every third-party GitHub Action must be pinned to a full commit SHA. A tag can be moved to
# point at different code; a commit cannot.
set -euo pipefail

cd "$(dirname "$0")/../.."
failed=0
while IFS= read -r line; do
  file=${line%%:*}
  ref=$(sed -E 's/.*uses:[[:space:]]*([^[:space:]#]+).*/\1/' <<<"$line")
  case "$ref" in
    ./*) continue ;; # local action or reusable workflow in this repository
  esac
  if ! grep -qE '@[0-9a-f]{40}$' <<<"$ref"; then
    echo "error: $file: '$ref' is not pinned to a commit SHA" >&2
    failed=1
  fi
done < <(grep -rnE '^[[:space:]]*-?[[:space:]]*uses:' .github/workflows || true)

[ "$failed" -eq 0 ] || exit 1
echo "All GitHub Actions are pinned to a commit SHA"
