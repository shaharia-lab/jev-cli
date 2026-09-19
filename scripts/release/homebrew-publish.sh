#!/usr/bin/env bash
# Writes the formulas of release <version> to the Homebrew tap and lists jev in the tap's README:
#
#   scripts/release/homebrew-publish.sh <version> <formula-dir>
#
# <formula-dir> holds jev.rb and jev@<version>.rb, as homebrew-formula.sh prints them. Each file
# is written through the GitHub contents API, one commit each, as the tap's other tools do:
#
#   Formula/jev@<version>.rb  the pinned formula
#   Formula/jev.rb            the main formula, unless the tap already points it at a newer release
#   README.md                 a `jev` row in the "Available Formulas" table
#
# Safe to run again: a file whose content is already right is left alone, so a re-run for the same
# release commits nothing, and re-running an older release never moves the main formula back.
# Needs `gh` with a token that may write the tap's contents (GH_TOKEN) and `jq`. HOMEBREW_TAP
# names the tap repository, shaharia-lab/homebrew-tap by default.
set -euo pipefail

if [ $# -ne 2 ]; then
  echo "usage: $0 <version> <formula-dir>" >&2
  exit 2
fi
version=$1 dir=$2
tap=${HOMEBREW_TAP:-shaharia-lab/homebrew-tap}
branch=main
row="| \`jev\` | Unofficial CLI for TypeSafe AI's Jev model, for humans, scripts and AI agents | [jev-cli](https://github.com/shaharia-lab/jev-cli) |"

for file in jev.rb "jev@$version.rb"; do
  [ -f "$dir/$file" ] || { echo "error: $dir/$file does not exist" >&2; exit 2; }
done
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Fetches <path> from the tap into $tmp/current and prints its blob sha. Returns 3 when the file
# does not exist, and 1 for any other failure.
fetch() {
  if ! gh api "repos/$tap/contents/$1?ref=$branch" > "$tmp/response" 2> "$tmp/error"; then
    grep -q 'HTTP 404' "$tmp/error" && return 3
    cat "$tmp/error" >&2
    return 1
  fi
  jq -r .content "$tmp/response" | base64 -d > "$tmp/current" || return 1
  jq -r .sha "$tmp/response"
}

# Writes <file> to <path> in the tap with commit <message>, unless it already has that content.
put() {
  local path=$1 file=$2 message=$3 sha code=0
  sha=$(fetch "$path") || code=$?
  case $code in
    0) if cmp -s "$file" "$tmp/current"; then
      echo "$path is already up to date"
      return 0
    fi ;;
    3) sha="" ;;
    *) exit 1 ;;
  esac
  local args=(-f "message=$message" -f "content=$(base64 < "$file" | tr -d '\n')" -f "branch=$branch")
  [ -z "$sha" ] || args+=(-f "sha=$sha")
  gh api --method PUT "repos/$tap/contents/$path" "${args[@]}" > /dev/null
  echo "Wrote $path"
}

put "Formula/jev@$version.rb" "$dir/jev@$version.rb" "Add jev@$version versioned formula"

code=0
fetch Formula/jev.rb > /dev/null || code=$?
current=""
case $code in
  0) current=$(grep -oE 'releases/download/v[^/"]+/' "$tmp/current" | head -n 1 | sed -E 's|^releases/download/v||; s|/$||' || true) ;;
  3) ;;
  *) exit 1 ;;
esac
if [ -n "$current" ] && [ "$current" != "$version" ] &&
  [ "$(printf '%s\n%s\n' "$current" "$version" | sort -V | tail -n 1)" = "$current" ]; then
  echo "::notice::Formula/jev.rb already points at the newer release $current; left as it is"
else
  put Formula/jev.rb "$dir/jev.rb" "jev $version"
fi

# The README row: replaced where it exists, otherwise added after the last row of the table that
# follows "## Available Formulas".
code=0
fetch README.md > /dev/null || code=$?
[ "$code" -eq 0 ] || exit 1
awk -v row="$row" '
  /^## / { section = ($0 == "## Available Formulas") }
  section && /^\| `jev` \|/ { $0 = row; found = 1 }
  section && /^\|/ { last = NR }
  { lines[NR] = $0 }
  END {
    if (!last) exit 3
    for (i = 1; i <= NR; i++) {
      print lines[i]
      if (i == last && !found) print row
    }
  }
' "$tmp/current" > "$tmp/README.md" || code=$?
if [ "$code" -ne 0 ]; then
  echo "::warning::the tap README has no \"## Available Formulas\" table; add jev to it by hand"
else
  put README.md "$tmp/README.md" "Add jev to the formula table"
fi
