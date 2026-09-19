#!/usr/bin/env bash
# Publishes release <version> of the two crates to crates.io, the library first:
#
#   scripts/release/crates-publish.sh <version>
#
#   jev-client  the library, which jev-cli depends on
#   jev-cli     the `jev` binary; once jev-client <version> is in the index
#
# A crates.io version can never be replaced or deleted, so this is safe to run again after a
# partial failure: a crate whose <version> is already in the index is skipped, not published
# twice. Stable versions only, and only the version in Cargo.toml.
#
# Needs cargo with a crates.io token allowed to publish these crates (CARGO_REGISTRY_TOKEN, which
# cargo reads itself and this script never prints), curl and jq. CRATES_INDEX names the sparse
# index, https://index.crates.io by default; CRATES_INDEX_WAIT the seconds to wait for a published
# version to appear in it (600).
set -euo pipefail

if [ $# -ne 1 ]; then
  echo "usage: $0 <version>" >&2
  exit 2
fi
version=$1
root=$(cd "$(dirname "$0")/../.." && pwd)
index=${CRATES_INDEX:-https://index.crates.io}
wait_seconds=${CRATES_INDEX_WAIT:-600}

if ! grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$' <<< "$version"; then
  echo "error: '$version' is not a stable SemVer version; pre-releases are never sent to crates.io" >&2
  exit 2
fi
manifest=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' "$root/Cargo.toml")
if [ "$manifest" != "$version" ]; then
  echo "error: Cargo.toml says version $manifest, not $version" >&2
  exit 1
fi
if [ -z "${CARGO_REGISTRY_TOKEN-}" ]; then
  echo "error: CARGO_REGISTRY_TOKEN is not set" >&2
  exit 1
fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cd "$root"

# Succeeds when <crate> <version> is in the index, returns 3 when it is not, and 1 when the index
# cannot be read. A sparse index file holds one JSON line per published version.
published() {
  local crate=$1 status
  # The index path of a name of four or more characters: its first two, its next two, itself.
  status=$(curl -sS --proto =https --retry 3 -o "$tmp/index" -w '%{http_code}' \
    "$index/${crate:0:2}/${crate:2:2}/$crate") || return 1
  case $status in
    200) ;;
    404) return 3 ;;
    *)
      echo "error: the crates.io index answered HTTP $status for $crate" >&2
      return 1
      ;;
  esac
  jq -en --arg version "$version" '[inputs | select(.vers == $version)] | length > 0' \
    < "$tmp/index" > /dev/null || return 3
}

for crate in jev-client jev-cli; do
  code=0
  published "$crate" || code=$?
  case $code in
    0)
      echo "::notice::$crate $version is already on crates.io; skipped"
      continue
      ;;
    3) ;;
    *) exit 1 ;;
  esac

  # cargo builds the packaged crate once more before uploading it, so nothing that fails to build
  # from crates.io is ever published.
  if ! cargo publish --locked -p "$crate" 2>&1 | tee "$tmp/publish"; then
    # The index lags behind uploads: an earlier run may have published it moments ago.
    if grep -qE 'already (exists|uploaded)' "$tmp/publish"; then
      echo "::notice::$crate $version is already on crates.io; skipped"
      continue
    fi
    echo "error: publishing $crate $version failed" >&2
    exit 1
  fi

  # jev-cli can only be published once cargo can resolve jev-client <version> from the index.
  waited=0
  until published "$crate"; do
    if [ "$waited" -ge "$wait_seconds" ]; then
      echo "error: $crate $version did not appear in the crates.io index within ${wait_seconds}s" >&2
      exit 1
    fi
    sleep 10
    waited=$((waited + 10))
  done
  echo "Published $crate $version"
done
