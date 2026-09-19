#!/usr/bin/env bash
# Checks the Homebrew tap plumbing without touching the tap:
#
#   scripts/release/homebrew-self-test.sh
#
# 1. homebrew-formula.sh puts each target's checksum from SHA256SUMS next to its archive, names
#    the pinned formula's class as Homebrew does, and refuses a pre-release main formula, a
#    missing archive and bad arguments.
# 2. homebrew-publish.sh, against a fake `gh` that keeps the tap in a directory: the first run
#    writes both formulas and the README row; a second run writes nothing; an older release adds
#    its pinned formula without moving the main formula back; a newer one moves it forward; an
#    existing README row is replaced, not repeated; an API failure fails the run.
#
# The formulas are checked by Homebrew itself (style, audit, install and test) in the release
# workflow's `homebrew-check` job, which needs real archives.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
formula="$root/scripts/release/homebrew-formula.sh"
publish="$root/scripts/release/homebrew-publish.sh"
failed=0
cases=0
fail() {
  echo "error: $*" >&2
  failed=1
}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# expect <exit-code> <text the output must contain> <description> <command...>
expect() {
  local want=$1 text=$2 description=$3 output code=0
  shift 3
  cases=$((cases + 1))
  output=$("$@" 2>&1) || code=$?
  if [ "$code" -ne "$want" ]; then
    fail "$description: exit $code, expected $want. Output:
$output"
  elif ! grep -qF -- "$text" <<< "$output"; then
    fail "$description: the output does not contain '$text'. Output:
$output"
  fi
}
# expect_file <file> <grep -E pattern> <expected number of matching lines> <description>
expect_file() {
  local count
  cases=$((cases + 1))
  count=$(grep -cE -- "$2" "$1" || true)
  [ "$count" = "$3" ] || fail "$4: $(basename "$1") has $count lines matching '$2', expected $3"
}

# ── 1. The formula ──────────────────────────────────────────────────────────────────────────────
sums() {
  local version=$1 target
  shift
  for target in "$@"; do
    echo "$(printf '%s' "$version $target" | sha256sum | cut -d' ' -f1)  jev-$version-$target.tar.gz"
  done
  echo "$(printf 'win' | sha256sum | cut -d' ' -f1) *jev-$version-x86_64-pc-windows-msvc.zip"
}
unix_targets=(aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-musl x86_64-unknown-linux-musl)
sums 0.12.0 "${unix_targets[@]}" > "$tmp/SHA256SUMS"

"$formula" main 0.12.0 "$tmp/SHA256SUMS" > "$tmp/jev.rb"
"$formula" pinned 0.12.0 "$tmp/SHA256SUMS" > "$tmp/pinned.rb"
expect_file "$tmp/jev.rb" '^class Jev < Formula$' 1 "the main formula's class"
expect_file "$tmp/jev.rb" 'keg_only' 0 "the main formula is linked"
expect_file "$tmp/pinned.rb" '^class JevAT0120 < Formula$' 1 "the pinned formula's class"
expect_file "$tmp/pinned.rb" '^  keg_only :versioned_formula$' 1 "the pinned formula is keg-only"
for file in "$tmp/jev.rb" "$tmp/pinned.rb"; do
  for target in "${unix_targets[@]}"; do
    sha=$(printf '%s' "0.12.0 $target" | sha256sum | cut -d' ' -f1)
    url="https://github.com/shaharia-lab/jev-cli/releases/download/v0.12.0/jev-0.12.0-$target.tar.gz"
    cases=$((cases + 1))
    grep -A1 -xF "      url \"$url\"" "$file" | grep -qxF "      sha256 \"$sha\"" ||
      fail "$(basename "$file") does not pair $target's url with its checksum"
  done
  expect_file "$file" 'windows' 0 "no Windows archive"
  expect_file "$file" '^    (bin|man1|bash_completion|zsh_completion|fish_completion)\.install ' 5 \
    "the binary, man pages and three completions are installed"
  expect_file "$file" 'jev version --field version' 1 "the test runs jev version"
done
"$formula" main 0.12.0 "$tmp/SHA256SUMS" file:///dist > "$tmp/local.rb"
expect_file "$tmp/local.rb" '^      url "file:///dist/jev-0\.12\.0-' 4 "a url base"

expect 1 "is a pre-release" "a pre-release main formula" "$formula" main 0.13.0-rc.1 "$tmp/SHA256SUMS"
sums 0.13.0-rc.1 "${unix_targets[@]}" > "$tmp/rc-sums"
expect 0 "class JevAT0130rc1 < Formula" "a pinned pre-release" "$formula" pinned 0.13.0-rc.1 "$tmp/rc-sums"
sums 0.12.0 aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-musl > "$tmp/partial"
expect 1 "has no checksum for jev-0.12.0-aarch64-unknown-linux-musl.tar.gz" "a missing archive" \
  "$formula" main 0.12.0 "$tmp/partial"
expect 2 "not a SemVer version" "a bad version" "$formula" main v0.12.0 "$tmp/SHA256SUMS"
expect 2 "must be main or pinned" "a bad kind" "$formula" latest 0.12.0 "$tmp/SHA256SUMS"
expect 2 "usage:" "no arguments" "$formula"

# ── 2. Publishing to a fake tap ─────────────────────────────────────────────────────────────────
# The fake `gh api` serves the tap from $FAKE_TAP like the contents API: GET returns the base64
# content and a sha, PUT needs the current sha to replace a file, and every write is logged.
mkdir -p "$tmp/bin" "$tmp/tap/Formula"
cat > "$tmp/bin/gh" << 'EOF'
#!/usr/bin/env bash
set -euo pipefail
[ "$1" = api ] || { echo "fake gh: unexpected $*" >&2; exit 64; }
shift
method=GET
declare -A field=()
while [ $# -gt 0 ]; do
  case $1 in
    --method) method=$2; shift 2 ;;
    -f) field[${2%%=*}]=${2#*=}; shift 2 ;;
    *) endpoint=$1; shift ;;
  esac
done
[ -z "${FAKE_FAIL-}" ] || { echo "gh: Server Error (HTTP 500)" >&2; exit 1; }
path=${endpoint#repos/shaharia-lab/homebrew-tap/contents/}
path=${path%%\?*}
file="$FAKE_TAP/$path"
if [ "$method" = GET ]; then
  [ -f "$file" ] || { echo "gh: Not Found (HTTP 404)" >&2; exit 1; }
  jq -n --arg content "$(base64 < "$file")" --arg sha "$(sha1sum < "$file" | cut -d' ' -f1)" \
    '{content: $content, sha: $sha}'
  exit 0
fi
if [ -f "$file" ] && [ "${field[sha]-}" != "$(sha1sum < "$file" | cut -d' ' -f1)" ]; then
  echo "gh: sha does not match (HTTP 409)" >&2; exit 1
fi
[ "${field[branch]}" = main ] || { echo "fake gh: branch ${field[branch]}" >&2; exit 64; }
printf '%s' "${field[content]}" | base64 -d > "$file"
echo "${field[message]}" >> "$FAKE_TAP.log"
echo '{}'
EOF
chmod +x "$tmp/bin/gh"
cat > "$tmp/tap/README.md" << 'EOF'
# Tap

## Available Formulas

| Formula | Description | Repository |
|---------|-------------|------------|
| `codenav` | Code | [code-navigator](https://github.com/shaharia-lab/code-navigator) |
| `vibexp` | VibeXP | [vibexp.io](https://github.com/shaharia-lab/vibexp.io) |

## Installation Methods

| Not | the | table |
EOF
cp "$tmp/tap/README.md" "$tmp/README.orig"

# formulas <version>: a formula directory for <version>, as the release workflow builds it.
formulas() {
  mkdir -p "$tmp/formulas-$1"
  sums "$1" "${unix_targets[@]}" > "$tmp/formulas-$1/SHA256SUMS"
  "$formula" main "$1" "$tmp/formulas-$1/SHA256SUMS" > "$tmp/formulas-$1/jev.rb"
  "$formula" pinned "$1" "$tmp/formulas-$1/SHA256SUMS" > "$tmp/formulas-$1/jev@$1.rb"
  echo "$tmp/formulas-$1"
}
run() { PATH="$tmp/bin:$PATH" FAKE_TAP="$tmp/tap" "$publish" "$@"; }
writes() { wc -l < "$tmp/tap.log" 2> /dev/null | tr -d ' ' || echo 0; }
main_points_at() {
  cases=$((cases + 1))
  grep -q "releases/download/v$1/" "$tmp/tap/Formula/jev.rb" ||
    fail "$2: Formula/jev.rb does not point at $1"
}

expect 0 "Wrote README.md" "the first release" run 0.12.0 "$(formulas 0.12.0)"
cases=$((cases + 1))
cmp -s "$tmp/tap/Formula/jev@0.12.0.rb" "$tmp/formulas-0.12.0/jev@0.12.0.rb" || fail "the pinned formula differs"
main_points_at 0.12.0 "the first release"
cases=$((cases + 1))
[ "$(writes)" = 3 ] || fail "the first release made $(writes) commits, expected 3"
cases=$((cases + 1))
diff <(grep -v '^| `jev`' "$tmp/tap/README.md") "$tmp/README.orig" > /dev/null ||
  fail "the README changed beyond the jev row"
cases=$((cases + 1))
grep -A1 -F '| `vibexp` |' "$tmp/tap/README.md" | grep -qF '| `jev` | Unofficial CLI' ||
  fail "the jev row does not follow the last row of the formula table"

expect 0 "README.md is already up to date" "a re-run" run 0.12.0 "$(formulas 0.12.0)"
cases=$((cases + 1))
[ "$(writes)" = 3 ] || fail "a re-run made commits"

expect 0 "already points at the newer release 0.12.0" "an older release" run 0.9.1 "$(formulas 0.9.1)"
main_points_at 0.12.0 "an older release"
cases=$((cases + 1))
[ -f "$tmp/tap/Formula/jev@0.9.1.rb" ] || fail "an older release did not add its pinned formula"

expect 0 "Wrote Formula/jev.rb" "a newer release" run 0.13.0 "$(formulas 0.13.0)"
main_points_at 0.13.0 "a newer release"

sed 's/^| `jev` |.*/| `jev` | An old description | [jev-cli](x) |/' "$tmp/tap/README.md" > "$tmp/edited"
mv "$tmp/edited" "$tmp/tap/README.md"
expect 0 "Wrote README.md" "an outdated README row" run 0.13.0 "$(formulas 0.13.0)"
expect_file "$tmp/tap/README.md" '^\| `jev` \|' 1 "the README row is replaced, not repeated"
expect_file "$tmp/tap/README.md" 'An old description' 0 "the README row is replaced"

printf '# Tap\n\nNo table.\n' > "$tmp/tap/README.md"
expect 0 "no \"## Available Formulas\" table" "a README without the table" run 0.13.0 "$(formulas 0.13.0)"

expect 1 "HTTP 500" "an API failure" env FAKE_FAIL=1 PATH="$tmp/bin:$PATH" FAKE_TAP="$tmp/tap" \
  "$publish" 0.13.0 "$tmp/formulas-0.13.0"
expect 2 "jev@0.14.0.rb does not exist" "a missing formula" run 0.14.0 "$tmp/formulas-0.13.0"

[ "$failed" -eq 0 ] || exit 1
echo "The Homebrew formula and tap publishing pass $cases cases"
