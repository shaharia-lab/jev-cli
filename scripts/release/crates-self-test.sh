#!/usr/bin/env bash
# Checks the crates.io publishing plumbing without touching crates.io:
#
#   scripts/release/crates-self-test.sh
#
# crates-publish.sh runs against a fake `cargo` that "publishes" into a fake sparse index, served
# by a fake `curl`: the first run publishes jev-client and then jev-cli; a second run publishes
# nothing; a run after a partial publish publishes only jev-cli; an upload that crates.io rejects
# as already there is skipped; a failed upload stops before jev-cli; an index error, a version
# that never reaches the index, a pre-release, a version Cargo.toml does not have and a missing
# token all fail. The token never reaches the output.
#
# The real packaging is checked by `cargo package --workspace` in CI.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
failed=0
cases=0
fail() {
  echo "error: $*" >&2
  failed=1
}
token=cio-sentinel-token-7f3a9c
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
  elif grep -qF -- "$token" <<< "$output"; then
    fail "$description: the output contains the token"
  fi
}

# A checkout whose Cargo.toml says a stable version, whatever the real one says today. `$run`
# publishes it with the fakes first on the PATH.
stable=0.12.0
mkdir -p "$tmp/checkout/scripts/release" "$tmp/bin" "$tmp/index"
cp "$root/scripts/release/crates-publish.sh" "$tmp/checkout/scripts/release/"
sed "/^\[workspace.package\]/,/^\[/s/^version *= *\".*\"/version = \"$stable\"/" "$root/Cargo.toml" \
  > "$tmp/checkout/Cargo.toml"
run=$tmp/run
cat > "$run" << RUN
#!/usr/bin/env bash
PATH="$tmp/bin:\$PATH" FAKE_INDEX="$tmp/index" CRATES_INDEX_WAIT=0 CARGO_REGISTRY_TOKEN=$token \\
  exec "$tmp/checkout/scripts/release/crates-publish.sh" "\$@"
RUN

# The fake `curl` serves $FAKE_INDEX/<name> for https://index.crates.io/<xx>/<yy>/<name>, as
# the sparse index does, and prints the HTTP status for -w.
cat > "$tmp/bin/curl" << 'CURL'
#!/usr/bin/env bash
set -euo pipefail
out="" url=""
while [ $# -gt 0 ]; do
  case $1 in
    -o) out=$2; shift 2 ;;
    -w | --proto | --retry) shift 2 ;;
    -*) shift ;;
    *) url=$1; shift ;;
  esac
done
[ -z "${FAKE_INDEX_STATUS-}" ] || { printf '%s' "$FAKE_INDEX_STATUS"; exit 0; }
case $url in
  https://index.crates.io/je/v-/jev-cli | https://index.crates.io/je/v-/jev-client) ;;
  *) echo "fake curl: unexpected $url" >&2; exit 64 ;;
esac
file="$FAKE_INDEX/${url##*/}"
if [ -f "$file" ]; then cp "$file" "$out"; printf 200; else printf 404; fi
CURL
# The fake `cargo publish`, run from the checkout, logs each upload and adds the version to the
# index, unless FAKE_PUBLISH says otherwise: `fail` rejects it, `exists` rejects it as crates.io
# does a duplicate, `unindexed` accepts it but never indexes it.
cat > "$tmp/bin/cargo" << 'CARGO'
#!/usr/bin/env bash
set -euo pipefail
[ "$1 $2 $3" = "publish --locked -p" ] || { echo "fake cargo: unexpected $*" >&2; exit 64; }
crate=$4
[ -n "${CARGO_REGISTRY_TOKEN-}" ] || { echo "fake cargo: no token" >&2; exit 64; }
version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' Cargo.toml)
case ${FAKE_PUBLISH-} in
  fail) echo "error: failed to verify package tarball" >&2; exit 101 ;;
  exists) echo "error: crate $crate@$version already exists on crates.io index" >&2; exit 101 ;;
esac
echo "$crate" >> "$FAKE_INDEX.log"
echo "   Uploaded $crate v$version"
[ "${FAKE_PUBLISH-}" = unindexed ] ||
  printf '{"name":"%s","vers":"%s","yanked":false}\n' "$crate" "$version" >> "$FAKE_INDEX/$crate"
CARGO
chmod +x "$run" "$tmp/bin/curl" "$tmp/bin/cargo"

uploads() { [ ! -f "$tmp/index.log" ] || tr '\n' ' ' < "$tmp/index.log" | sed 's/ $//'; }
reset() { rm -rf "$tmp/index" "$tmp/index.log" && mkdir "$tmp/index"; }
uploaded() {
  cases=$((cases + 1))
  [ "$(uploads)" = "$1" ] || fail "$2: uploaded '$(uploads)', expected '$1'"
}

printf '{"name":"jev-client","vers":"0.11.0","yanked":false}\n' > "$tmp/index/jev-client"
expect 0 "Published jev-cli $stable" "the first release" "$run" "$stable"
uploaded "jev-client jev-cli" "the first release"

expect 0 "jev-cli $stable is already on crates.io; skipped" "a re-run" "$run" "$stable"
uploaded "jev-client jev-cli" "a re-run"

reset
printf '{"name":"jev-client","vers":"%s","yanked":false}\n' "$stable" > "$tmp/index/jev-client"
expect 0 "jev-client $stable is already on crates.io; skipped" "after a partial publish" "$run" "$stable"
uploaded "jev-cli" "after a partial publish"

reset
expect 0 "jev-cli $stable is already on crates.io; skipped" "an upload the index does not show yet" \
  env FAKE_PUBLISH=exists "$run" "$stable"
uploaded "" "an upload the index does not show yet"

reset
expect 1 "publishing jev-client $stable failed" "a failed upload" env FAKE_PUBLISH=fail "$run" "$stable"
uploaded "" "a failed upload"

reset
expect 1 "did not appear in the crates.io index" "a version that never reaches the index" \
  env FAKE_PUBLISH=unindexed "$run" "$stable"
uploaded "jev-client" "a version that never reaches the index"

reset
expect 1 "answered HTTP 503" "an index error" env FAKE_INDEX_STATUS=503 "$run" "$stable"
uploaded "" "an index error"

expect 2 "never sent to crates.io" "a pre-release" "$run" 0.13.0-rc.1
expect 1 "Cargo.toml says version $stable, not 0.13.0" "another version" "$run" 0.13.0
expect 1 "CARGO_REGISTRY_TOKEN is not set" "no token" \
  env -u CARGO_REGISTRY_TOKEN PATH="$tmp/bin:$PATH" "$tmp/checkout/scripts/release/crates-publish.sh" "$stable"
expect 2 "usage:" "no arguments" "$run"
uploaded "" "the refused runs"

[ "$failed" -eq 0 ] || exit 1
echo "crates.io publishing passes $cases cases"
