#!/usr/bin/env bash
# Uploads the verified assets in <dir> to the GitHub Release of <tag>, as a draft:
#
#   scripts/release/publish.sh <tag> <dir>
#
# Safe to run again. The release is created as a draft when it does not exist (release-please
# creates it as a draft itself). While it is a draft, its assets are made exactly the files in
# <dir>: each is uploaded, replacing one of the same name, and any other asset is deleted, so a
# re-run cannot leave a mix of two attempts. A release that is no longer a draft has passed
# verification and been published; its assets are never touched again, and this does nothing.
# Needs `gh` with a token that may write releases (GH_TOKEN) and GH_REPO set.
set -euo pipefail

if [ $# -ne 2 ]; then
  echo "usage: $0 <tag> <dir>" >&2
  exit 2
fi
tag=$1 dir=$2

if ! gh release view "$tag" --json isDraft > /dev/null 2>&1; then
  echo "Creating draft release $tag" >&2
  gh release create "$tag" --draft --verify-tag --title "$tag" --generate-notes > /dev/null
fi

if [ "$(gh release view "$tag" --json isDraft --jq .isDraft)" != true ]; then
  echo "::notice::$tag is already published; its assets are left as they are"
  exit 0
fi

mapfile -t files < <(find "$dir" -maxdepth 1 -type f | LC_ALL=C sort)
gh release upload "$tag" "${files[@]}" --clobber

wanted=$(for file in "${files[@]}"; do basename "$file"; done)
while IFS= read -r asset; do
  if [ -n "$asset" ] && ! grep -qxF "$asset" <<< "$wanted"; then
    echo "Deleting stale asset $asset" >&2
    gh release delete-asset "$tag" "$asset" --yes
  fi
done < <(gh release view "$tag" --json assets --jq '.assets[].name')

echo "Uploaded ${#files[@]} assets to draft release $tag" >&2
