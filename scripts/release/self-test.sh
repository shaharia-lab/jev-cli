#!/usr/bin/env bash
# Checks the release-signing plumbing without the real signing key:
#
#   scripts/release/self-test.sh
#
# 1. The committed public keys (crates/jev-cli/keys) are well-formed minisign keys whose key ids
#    match their comments, differ from each other, and are the ones SECURITY.md tells people to
#    verify with; and no minisign secret key is tracked by git.
# 2. With a throwaway key pair: sign.sh signs a fake release, verify-assets.sh accepts it, and
#    rejects every kind of tampering (an altered asset, a regenerated SHA256SUMS, another key, a
#    missing, swapped or replayed signature). The signing password never reaches the output.
#
# Part 2 needs minisign, tar, zip and unzip. Without minisign it is skipped with a warning, except
# in CI (CI=true), where it must run.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
keys="$root/crates/jev-cli/keys"
failed=0
fail() {
  echo "error: $*" >&2
  failed=1
}

# ── 1. The committed public keys ────────────────────────────────────────────────────────────────
ids=()
for role in primary next; do
  file="$keys/release-$role.pub"
  if [ ! -f "$file" ]; then
    fail "$file does not exist"
    continue
  fi
  comment=$(sed -n 1p "$file")
  encoded=$(sed -n 2p "$file")
  id=${comment#untrusted comment: minisign public key }
  [ "$(wc -l < "$file" | tr -d ' ')" = 2 ] || fail "$file must have exactly two lines"
  grep -qE '^[0-9A-F]{16}$' <<< "$id" || fail "$file: unexpected first line '$comment'"
  # A minisign public key is "Ed", the 8-byte key id (little-endian), and the 32-byte Ed25519 key.
  bytes=$(base64 -d <<< "$encoded" 2> /dev/null | od -An -v -tx1 | tr -d ' \n' || true)
  if [ "${#bytes}" -ne 84 ] || [ "${bytes:0:4}" != 4564 ]; then
    fail "$file: the second line is not a minisign Ed25519 public key"
  else
    decoded_id=""
    for ((i = 18; i >= 4; i -= 2)); do decoded_id+=${bytes:i:2}; done
    [ "${decoded_id^^}" = "$id" ] || fail "$file: the comment says key id $id, the key is ${decoded_id^^}"
  fi
  grep -qF "$encoded" "$root/SECURITY.md" || fail "SECURITY.md does not list the $role key $encoded"
  grep -qF "$id" "$root/SECURITY.md" || fail "SECURITY.md does not name the $role key id $id"
  ids+=("$id")
done
[ "${#ids[@]}" -ne 2 ] || [ "${ids[0]}" != "${ids[1]}" ] || fail "the primary and next keys are the same key"

if leaked=$(git -C "$root" grep -lE '^untrusted comment: minisign .*secret key' -- . 2> /dev/null); then
  fail "a minisign secret key is tracked by git: $leaked"
fi

if ! command -v minisign > /dev/null; then
  if [ "${CI-}" = true ]; then
    fail "minisign is not installed"
  else
    echo "warning: minisign is not installed; skipping the signing round trip" >&2
  fi
  [ "$failed" -eq 0 ] || exit 1
  echo "The release public keys are well-formed and documented"
  exit 0
fi
[ "$failed" -eq 0 ] || exit 1

# ── 2. Signing and verification with a throwaway key ───────────────────────────────────────────
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
version=0.0.0-selftest
password="self-test-password-$$-$RANDOM"
printf '%s\n%s\n' "$password" "$password" | minisign -G -p "$tmp/test.pub" -s "$tmp/test.key" > /dev/null 2>&1
printf 'other\nother\n' | minisign -G -p "$tmp/other.pub" -s "$tmp/other.key" > /dev/null 2>&1

# A fake release: one archive per target with every entry verify-assets.sh looks for.
dist="$tmp/dist"
mkdir -p "$dist"
targets=$(sed -n '/^TARGETS=(/,/^)/p' "$root/scripts/release/verify-assets.sh" | sed '1d;$d' | tr -d ' ')
for target in $targets; do
  name="jev-$version-$target"
  stage="$tmp/stage/$name"
  mkdir -p "$stage/man/man1" "$stage/completions"
  for entry in jev jev.exe README.md LICENSE-MIT LICENSE-APACHE man/man1/jev.1 completions/jev.bash \
    completions/_jev completions/jev.fish completions/_jev.ps1; do
    echo "$target $entry" > "$stage/$entry"
  done
  case $target in
    *-windows-*) (cd "$tmp/stage" && zip -qr "$dist/$name.zip" "$name") ;;
    *) tar -C "$tmp/stage" -czf "$dist/$name.tar.gz" "$name" ;;
  esac
done
cp "$root"/schemas/*.schema.json "$dist/"
echo '{"bomFormat":"CycloneDX"}' > "$dist/jev-$version.cdx.json"
(cd "$dist" && sha256sum -- * > SHA256SUMS)
archive=$(cd "$dist" && find . -name '*-x86_64-unknown-linux-musl.tar.gz' | sed 's|^\./||')

cases=0
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
  elif grep -qF -- "$password" <<< "$output"; then
    fail "$description: the output contains the signing password"
  fi
}
verify() { "$root/scripts/release/verify-assets.sh" "$@"; }
sign() {
  MINISIGN_SECRET_KEY=$(cat "$2") MINISIGN_PASSWORD=$3 "$root/scripts/release/sign.sh" "$1" "$version"
}
# A copy of the signed release, to tamper with.
copy() {
  rm -rf "$tmp/copy"
  cp -R "$dist" "$tmp/copy"
  echo "$tmp/copy"
}

expect 0 "present and match SHA256SUMS" "unsigned assets before signing" verify "$dist" "$version"
expect 1 "could not decrypt" "a wrong password" sign "$dist" "$tmp/test.key" "not-$password"
expect 2 "must both be set" "no password" sign "$dist" "$tmp/test.key" ""
[ -z "$(find "$dist" -name '*.minisig')" ] || fail "a failed signing run left signatures behind"
expect 1 "signatures differ" "unsigned assets checked against a key" verify "$dist" "$version" "$tmp/test.pub"
expect 0 "Signed $(find "$dist" -type f | wc -l | tr -d ' ') files" "signing" sign "$dist" "$tmp/test.key" "$password"
expect 0 "signed by test.pub" "the signed release" verify "$dist" "$version" "$tmp/test.pub"
expect 0 "Signature and comment signature verified" "the standard CLI, as SECURITY.md documents it" \
  minisign -Vm "$dist/$archive" -P "$(sed -n 2p "$tmp/test.pub")"
expect 1 "unexpected signatures before signing" "signatures where none are expected" verify "$dist" "$version"

dir=$(copy) && echo x >> "$dir/$archive"
expect 1 "$archive.minisig does not verify" "an altered archive" verify "$dir" "$version" "$tmp/test.pub"

dir=$(copy) && echo x >> "$dir/$archive" && (cd "$dir" && sha256sum -- $(sed -E 's/^[0-9a-f]{64} [ *]//' SHA256SUMS) > SHA256SUMS)
expect 1 "SHA256SUMS.minisig does not verify" "an altered archive with a regenerated SHA256SUMS" \
  verify "$dir" "$version" "$tmp/test.pub"

dir=$(copy) && rm "$dir"/*.minisig && sign "$dir" "$tmp/other.key" other 2> /dev/null
expect 1 "does not verify against test.pub" "a release signed by another key" verify "$dir" "$version" "$tmp/test.pub"

dir=$(copy) && rm "$dir/$archive.minisig"
expect 1 "signatures differ" "a missing signature" verify "$dir" "$version" "$tmp/test.pub"

dir=$(copy) && cp "$dir/SHA256SUMS.minisig" "$dir/$archive.minisig"
expect 1 "$archive.minisig does not verify" "a signature of another file" verify "$dir" "$version" "$tmp/test.pub"

dir=$(copy) && printf '%s\n' "$password" |
  minisign -S -s "$tmp/test.key" -m "$dir/SHA256SUMS" -t "$(printf 'file:SHA256SUMS\tversion:0.0.1')" > /dev/null 2>&1
expect 1 "a valid signature for a different file or release" "a signature replayed from another release" \
  verify "$dir" "$version" "$tmp/test.pub"

dir=$(copy) && cp "$dir/SHA256SUMS.minisig" "$dir/extra.minisig"
expect 1 "signatures differ" "an unexpected signature" verify "$dir" "$version" "$tmp/test.pub"

[ "$failed" -eq 0 ] || exit 1
echo "The release public keys are well-formed and documented; signing and verification pass $cases cases"
