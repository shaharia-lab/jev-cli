#!/usr/bin/env bash
# Checks that a directory holds exactly the assets of release <version>, intact:
#
#   scripts/release/verify-assets.sh <dir> <version>
#
# - one archive per target in TARGETS, the JSON Schemas from schemas/, and SHA256SUMS;
# - SHA256SUMS lists every other asset, and nothing else, and every checksum matches;
# - every archive holds the binary, README.md, both licences, completions and man pages.
#
# The release workflow runs it on the assembled assets before publishing and again on the assets
# downloaded from the GitHub Release, so nothing downstream can point at a missing or altered file.
set -euo pipefail

# Must match the build matrix in .github/workflows/release.yml.
TARGETS=(
  x86_64-unknown-linux-musl
  aarch64-unknown-linux-musl
  x86_64-apple-darwin
  aarch64-apple-darwin
  x86_64-pc-windows-msvc
  aarch64-pc-windows-msvc
)

if [ $# -ne 2 ]; then
  echo "usage: $0 <dir> <version>" >&2
  exit 2
fi
dir=$1 version=$2
root=$(cd "$(dirname "$0")/../.." && pwd)
failed=0
fail() {
  echo "error: $*" >&2
  failed=1
}

expected=()
for target in "${TARGETS[@]}"; do
  case $target in
    *-windows-*) expected+=("jev-$version-$target.zip") ;;
    *) expected+=("jev-$version-$target.tar.gz") ;;
  esac
done
for schema in "$root"/schemas/*.schema.json; do
  expected+=("$(basename "$schema")")
done

cd "$dir"
if [ ! -f SHA256SUMS ]; then
  echo "error: $dir has no SHA256SUMS" >&2
  exit 1
fi

# The three sets must be equal: what a release must contain, what is here, what SHA256SUMS lists.
want=$(printf '%s\n' "${expected[@]}" | LC_ALL=C sort)
have=$(find . -maxdepth 1 -type f ! -name SHA256SUMS | sed 's|^\./||' | LC_ALL=C sort)
listed=$(sed -E 's/^[0-9a-f]{64} [ *]//' SHA256SUMS | LC_ALL=C sort)
[ "$want" = "$have" ] || fail "assets differ from the expected set (< expected, > present):
$(diff <(echo "$want") <(echo "$have") | grep '^[<>]' || true)"
[ "$want" = "$listed" ] || fail "SHA256SUMS does not list exactly the expected assets (< expected, > listed):
$(diff <(echo "$want") <(echo "$listed") | grep '^[<>]' || true)"

if command -v sha256sum > /dev/null; then
  sha256sum --check --strict --quiet SHA256SUMS || fail "a checksum in SHA256SUMS does not match"
else
  shasum -a 256 --check --strict --quiet SHA256SUMS || fail "a checksum in SHA256SUMS does not match"
fi

for target in "${TARGETS[@]}"; do
  name="jev-$version-$target"
  case $target in
    *-windows-*) archive="$name.zip" exe=jev.exe ;;
    *) archive="$name.tar.gz" exe=jev ;;
  esac
  [ -f "$archive" ] || continue
  if [ "${archive##*.}" = zip ]; then
    entries=$(unzip -Z1 "$archive")
  else
    entries=$(tar -tzf "$archive")
  fi
  for entry in "$exe" README.md LICENSE-MIT LICENSE-APACHE man/man1/jev.1 \
    completions/jev.bash completions/_jev completions/jev.fish completions/_jev.ps1; do
    grep -qxF "$name/$entry" <<< "$entries" || fail "$archive does not contain $name/$entry"
  done
done

if [ "$failed" -ne 0 ]; then
  exit 1
fi
echo "All ${#expected[@]} assets of $version are present and match SHA256SUMS"
