#!/usr/bin/env bash
# Signs every file in <dir> with minisign, writing <file>.minisig beside it:
#
#   MINISIGN_SECRET_KEY=... MINISIGN_PASSWORD=... scripts/release/sign.sh <dir> <version>
#
# MINISIGN_SECRET_KEY is the content of a minisign secret key file, MINISIGN_PASSWORD its password.
# Each signature's trusted comment is `file:<name>\tversion:<version>`, so a signature cannot be
# passed off for another file or another release (SHA256SUMS has the same name in every release).
# verify-assets.sh checks it.
#
# Neither secret is ever passed as a command-line argument or printed: the key is written to a
# private temporary file that is removed on exit, and the password reaches minisign on stdin
# through `printf`, a shell builtin. Never run this with `set -x`. The password is used once, to
# decrypt that temporary copy (minisign's key derivation is deliberately slow), and the copy then
# signs every file.
set -euo pipefail
set +x

if [ $# -ne 2 ]; then
  echo "usage: MINISIGN_SECRET_KEY=... MINISIGN_PASSWORD=... $0 <dir> <version>" >&2
  exit 2
fi
dir=$1 version=$2

if [ -z "${MINISIGN_SECRET_KEY-}" ] || [ -z "${MINISIGN_PASSWORD-}" ]; then
  echo "error: MINISIGN_SECRET_KEY and MINISIGN_PASSWORD must both be set" >&2
  exit 2
fi

mapfile -t files < <(find "$dir" -maxdepth 1 -type f ! -name '*.minisig' | LC_ALL=C sort)
if [ ${#files[@]} -eq 0 ]; then
  echo "error: $dir has no files to sign" >&2
  exit 1
fi

key=$(umask 077 && mktemp)
trap 'rm -f "$key"' EXIT
printf '%s\n' "$MINISIGN_SECRET_KEY" > "$key"
# minisign's own output never includes the password; it is discarded to keep the log readable.
if ! printf '%s\n' "$MINISIGN_PASSWORD" | minisign -C -W -s "$key" > /dev/null 2>&1; then
  echo "error: could not decrypt MINISIGN_SECRET_KEY (is MINISIGN_PASSWORD its password?)" >&2
  exit 1
fi

for file in "${files[@]}"; do
  name=$(basename "$file")
  if ! minisign -S -W -s "$key" -m "$file" -t "$(printf 'file:%s\tversion:%s' "$name" "$version")" \
    < /dev/null > /dev/null; then
    echo "error: could not sign $name" >&2
    exit 1
  fi
  echo "Signed $name" >&2
done
echo "Signed ${#files[@]} files in $dir" >&2
