#!/usr/bin/env bash
# Packs one target's release archive and prints its path on stdout.
#
#   scripts/release/package.sh <version> <target> <binary> <dist-assets-dir> <out-dir>
#
# The archive is `jev-<version>-<target>.zip` for Windows targets and `.tar.gz` otherwise. It holds
# one directory of the same name with the binary, README.md, both licences, and the completions
# and man pages that `make dist-assets` wrote into <dist-assets-dir>.
set -euo pipefail

if [ $# -ne 5 ]; then
  echo "usage: $0 <version> <target> <binary> <dist-assets-dir> <out-dir>" >&2
  exit 2
fi
version=$1 target=$2 binary=$3 assets=$4 out=$5
root=$(cd "$(dirname "$0")/../.." && pwd)

case $target in
  *-windows-*) exe=jev.exe ext=zip ;;
  *) exe=jev ext=tar.gz ;;
esac
for required in "$binary" "$assets/completions" "$assets/man/man1/jev.1"; do
  if [ ! -e "$required" ]; then
    echo "error: $required does not exist" >&2
    exit 1
  fi
done

name="jev-$version-$target"
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
mkdir -p "$staging/$name" "$out"
cp "$binary" "$staging/$name/$exe"
cp "$root/README.md" "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$staging/$name/"
cp -R "$assets/completions" "$assets/man" "$staging/$name/"
chmod 0755 "$staging/$name/$exe"

archive="$(cd "$out" && pwd)/$name.$ext"
rm -f "$archive"
if [ "$ext" = tar.gz ]; then
  tar -C "$staging" -czf "$archive" "$name"
elif command -v 7z > /dev/null; then
  (cd "$staging" && 7z a -tzip -bso0 -bsp0 "$archive" "$name")
elif command -v zip > /dev/null; then
  (cd "$staging" && zip -qr "$archive" "$name")
else
  # Windows images without 7-Zip: PowerShell needs Windows paths.
  pwsh -NoProfile -Command "Compress-Archive -Path '$(cygpath -w "$staging/$name")' -DestinationPath '$(cygpath -w "$archive")'"
fi
echo "$archive"
