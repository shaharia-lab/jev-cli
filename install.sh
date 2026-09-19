#!/bin/sh
# Installs jev, an unofficial command-line tool for TypeSafe AI's Jev model, on Linux and macOS:
#
#   curl -fsSL https://raw.githubusercontent.com/shaharia-lab/jev-cli/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/shaharia-lab/jev-cli/main/install.sh | sh -s -- --version 0.1.0
#
# It downloads the archive for this platform from the project's GitHub Releases, checks its SHA-256
# against SHA256SUMS and, when minisign is installed, the minisign signatures of both against the
# release keys below, runs the new binary once, and puts it in a directory you own
# (~/.local/bin by default). It never uses sudo. `sh install.sh --help` lists the options.
#
# Windows: install.ps1. Everything else: cargo install jev-cli --locked.
set -eu

# ── Where releases come from, and which keys may sign them ─────────────────────────────────────
# One assignment per line: crates/jev-cli/tests/install_script.rs replaces these to install from a
# local server with a key of its own.
REPOSITORY_URL='https://github.com/shaharia-lab/jev-cli'
API_URL='https://api.github.com/repos/shaharia-lab/jev-cli'
ALLOW_HTTP=0
MINISIGN=minisign
# The release public keys: primary, then next (SECURITY.md, crates/jev-cli/keys). A release signed
# by either is accepted, as `jev update` accepts it.
PUBLIC_KEYS='RWS4h5k7TFvpPJt8wAoJCpoWgejSO8eVKRs+CqEdFIQAP2E0QwyyDDcU RWQIs3BoLNvYo0e6bXIBfp7hoYpwkQSJuAWcSD7itrj0ULCXWULmDMx4'

RAW_URL='https://raw.githubusercontent.com/shaharia-lab/jev-cli/main'
VERSION_PATTERN='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$'

usage() {
  cat << EOF
Install jev from GitHub Releases into a directory you own, verified. Never uses sudo.

Usage: install.sh [options]
       curl -fsSL $RAW_URL/install.sh | sh -s -- [options]

Options:
  --version <x.y.z>     Install this version, pre-releases (0.1.0-rc.1) included.
                        Default: the latest stable release.        [env: JEV_INSTALL_VERSION]
  --install-dir <dir>   Where to put jev. Default: ~/.local/bin.  [env: JEV_INSTALL_DIR]
  --no-modify-path      Do not add the directory to PATH in your shell profile; only say how.
                                                               [env: JEV_INSTALL_NO_MODIFY_PATH=1]
  --require-signature   Fail unless the signature is verified, which needs minisign on PATH.
                                                               [env: JEV_INSTALL_REQUIRE_SIGNATURE=1]
  -h, --help            Print this help.

What is verified: the archive's SHA-256 against SHA256SUMS, always; and, when minisign is on
PATH, the minisign signatures of the archive and of SHA256SUMS by a jev release key, each made
for that file of that version. The new binary must run and report the version asked for.
Nothing is installed unless every check passes.

The directory gets jev and .jev-update/receipt.json, which marks the install as self-managed so
that \`jev update\` can update it. Messages go to stderr; nothing is printed on stdout.

Exit status: 0 installed, 1 failed (nothing was installed), 2 usage error.

Examples:
  curl -fsSL $RAW_URL/install.sh | sh
  curl -fsSL $RAW_URL/install.sh | sh -s -- --version 0.1.0-rc.1
  curl -fsSL $RAW_URL/install.sh | sh -s -- --install-dir ~/bin --no-modify-path
EOF
}

say() {
  printf '%s\n' "$*" >&2
}

die() {
  say "error: $*"
  exit 1
}

usage_error() {
  say "error: $*"
  say "Run 'sh install.sh --help' for the options."
  exit 2
}

# ── Options ────────────────────────────────────────────────────────────────────────────────────
version=${JEV_INSTALL_VERSION-}
install_dir=${JEV_INSTALL_DIR-}
no_modify_path=${JEV_INSTALL_NO_MODIFY_PATH-}
require_signature=${JEV_INSTALL_REQUIRE_SIGNATURE-}
while [ $# -gt 0 ]; do
  case $1 in
    --version)
      [ $# -ge 2 ] || usage_error "--version needs a value, such as --version 0.1.0"
      version=$2
      shift 2
      ;;
    --version=*)
      version=${1#*=}
      shift
      ;;
    --install-dir)
      [ $# -ge 2 ] || usage_error "--install-dir needs a directory"
      install_dir=$2
      shift 2
      ;;
    --install-dir=*)
      install_dir=${1#*=}
      shift
      ;;
    --no-modify-path)
      no_modify_path=1
      shift
      ;;
    --require-signature)
      require_signature=1
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *) usage_error "unknown option '$1'" ;;
  esac
done
case $no_modify_path in "" | 0 | false) no_modify_path=0 ;; *) no_modify_path=1 ;; esac
case $require_signature in "" | 0 | false) require_signature=0 ;; *) require_signature=1 ;; esac
version=${version#v}
if [ -n "$version" ] && ! printf '%s\n' "$version" |
  grep -Eq "$VERSION_PATTERN"; then
  usage_error "'$version' is not a version such as 0.1.0 or 0.1.0-rc.1"
fi
if [ -z "$install_dir" ]; then
  [ -n "${HOME-}" ] || usage_error "HOME is not set; choose a directory with --install-dir"
  install_dir=$HOME/.local/bin
fi
case $install_dir in
  /*) ;;
  *) install_dir=$(pwd)/$install_dir ;;
esac

# ── Platform ───────────────────────────────────────────────────────────────────────────────────
unsupported() {
  die "there is no prebuilt jev for $1. Build it from source with Rust instead:
  cargo install jev-cli --locked"
}

os=$(uname -s)
arch=$(uname -m)
case $arch in
  x86_64 | amd64) arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
  *) unsupported "the $arch architecture" ;;
esac
case $os in
  Linux) target=$arch-unknown-linux-musl ;;
  Darwin)
    # A shell under Rosetta reports x86_64 on Apple silicon; the native binary is the better choice.
    if [ "$arch" = x86_64 ] && [ "$(sysctl -n hw.optional.arm64 2> /dev/null || true)" = 1 ]; then
      arch=aarch64
    fi
    target=$arch-apple-darwin
    ;;
  MINGW* | MSYS* | CYGWIN* | Windows*)
    die "this script is for Linux and macOS. On Windows, run in PowerShell:
  irm $RAW_URL/install.ps1 | iex"
    ;;
  *) unsupported "$os" ;;
esac

# ── Downloads (HTTPS only) ─────────────────────────────────────────────────────────────────────
if [ "$ALLOW_HTTP" = 1 ]; then protocols='=http,https'; else protocols='=https'; fi
if command -v curl > /dev/null 2>&1; then
  downloader=curl
elif command -v wget > /dev/null 2>&1 && wget --help 2>&1 | grep -q -- '--https-only'; then
  downloader=wget
else
  die "downloading needs curl (or GNU wget); install curl and run this again"
fi

# download <url> <file>: fails, printing nothing, when the server does not answer 2xx.
download() {
  case $1 in
    https://*) ;;
    http://*) [ "$ALLOW_HTTP" = 1 ] || die "refusing $1: only https:// is allowed" ;;
    *) die "refusing $1: only https:// is allowed" ;;
  esac
  if [ "$downloader" = curl ]; then
    curl --proto "$protocols" --proto-redir "$protocols" --tlsv1.2 --fail --silent --show-error \
      --location --retry 3 --connect-timeout 30 --output "$2" "$1" 2> "$tmp/download.log"
  elif [ "$ALLOW_HTTP" = 1 ]; then
    wget --quiet --tries=3 --output-document="$2" "$1" 2> "$tmp/download.log"
  else
    wget --quiet --https-only --tries=3 --output-document="$2" "$1" 2> "$tmp/download.log"
  fi
}

tmp=$(mktemp -d 2> /dev/null || mktemp -d -t jev-install)
staged=
cleanup() {
  rm -rf "$tmp"
  [ -z "$staged" ] || rm -f "$staged"
}
trap cleanup EXIT
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM

if [ -z "$version" ]; then
  if ! download "$API_URL/releases/latest" "$tmp/latest.json"; then
    die "could not find the latest stable release of jev ($(cat "$tmp/download.log")).
There may be no stable release yet: install a pre-release with --version <x.y.z-rc.N>
(see $REPOSITORY_URL/releases)."
  fi
  version=$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v\{0,1\}\([^"]*\)".*/\1/p' "$tmp/latest.json" | head -n 1)
  printf '%s\n' "$version" | grep -Eq "$VERSION_PATTERN" ||
    die "could not read the latest version from $API_URL/releases/latest; pass --version <x.y.z>"
fi

archive=jev-$version-$target.tar.gz
base=$REPOSITORY_URL/releases/download/v$version
say "Downloading jev $version for $target"
for file in SHA256SUMS SHA256SUMS.minisig "$archive" "$archive.minisig"; do
  if ! download "$base/$file" "$tmp/$file"; then
    die "could not download $base/$file ($(cat "$tmp/download.log")).
Check that jev $version exists and has a $target archive: $REPOSITORY_URL/releases"
  fi
done

# ── Verification ───────────────────────────────────────────────────────────────────────────────
# verify_signature <name>: the signature of $tmp/<name> is by a release key, for <name> and $version.
verify_signature() {
  expected_comment=$(printf 'file:%s\tversion:%s' "$1" "$version")
  for key in $PUBLIC_KEYS; do
    # -H: only prehashed signatures, which minisign makes. -Q prints the trusted comment, and only
    # when the signature is valid.
    if comment=$("$MINISIGN" -V -H -Q -P "$key" -m "$tmp/$1" -x "$tmp/$1.minisig" 2> /dev/null); then
      [ "$comment" = "$expected_comment" ] ||
        die "$1.minisig is a valid signature, but for another file or release (its trusted comment is '$comment'). Nothing was installed."
      return 0
    fi
  done
  die "$1.minisig is not a valid signature of $1 by a jev release key. Nothing was installed."
}

if command -v "$MINISIGN" > /dev/null 2>&1; then
  verify_signature SHA256SUMS
  verify_signature "$archive"
  signature="the minisign signatures of the archive and SHA256SUMS by a jev release key"
elif [ "$require_signature" = 1 ]; then
  die "--require-signature was given, but minisign is not installed (https://jedisct1.github.io/minisign/). Nothing was installed."
else
  signature=
fi

sha256() {
  if command -v sha256sum > /dev/null 2>&1; then
    sha256sum "$1"
  elif command -v shasum > /dev/null 2>&1; then
    shasum -a 256 "$1"
  elif command -v openssl > /dev/null 2>&1; then
    openssl dgst -sha256 -r "$1"
  else
    die "computing a SHA-256 needs sha256sum, shasum or openssl"
  fi
}
expected=$(awk -v name="$archive" '$2 == name || $2 == "*" name { print $1 }' "$tmp/SHA256SUMS")
actual=$(sha256 "$tmp/$archive" | awk '{ print $1 }')
# Exactly one line: 64 hex digits, with no second checksum after a newline.
case $expected in
  *[!0-9a-f]* | "") die "SHA256SUMS does not list $archive exactly once. Nothing was installed." ;;
  *) [ ${#expected} -eq 64 ] || die "SHA256SUMS does not list $archive exactly once. Nothing was installed." ;;
esac
[ "$actual" = "$expected" ] ||
  die "the SHA-256 of $archive does not match SHA256SUMS: the download is corrupt or was altered. Nothing was installed."

# ── Install ────────────────────────────────────────────────────────────────────────────────────
mkdir -p "$tmp/extract"
tar -xzf "$tmp/$archive" -C "$tmp/extract" "jev-$version-$target/jev" 2> /dev/null ||
  die "$archive does not contain jev-$version-$target/jev"
[ -f "$tmp/extract/jev-$version-$target/jev" ] || die "$archive does not contain the jev binary"

previous=$(command -v jev 2> /dev/null || true)
mkdir -p "$install_dir" 2> /dev/null ||
  die "could not create $install_dir; choose another directory with --install-dir (this script never uses sudo)"
[ -w "$install_dir" ] ||
  die "$install_dir is not writable by you; choose another directory with --install-dir (this script never uses sudo)"
[ ! -d "$install_dir/jev" ] || die "$install_dir/jev is a directory"

# Staged in the install directory, so that it can run even when the temporary directory is
# mounted noexec, and so that the final rename is atomic.
staged=$install_dir/.jev-install.$$
cp "$tmp/extract/jev-$version-$target/jev" "$staged"
chmod 0755 "$staged"
if ! reported=$(JEV_CONFIG_DIR="$tmp/config" "$staged" version --field version 2> "$tmp/self-test.log"); then
  die "the downloaded jev does not run on this system: $(cat "$tmp/self-test.log"). Nothing was installed."
fi
[ "$reported" = "$version" ] ||
  die "the downloaded jev reports version '$reported', not $version. Nothing was installed."
mv -f "$staged" "$install_dir/jev"
staged=

# The receipt tells `jev update` that this install is self-managed, so it may replace the binary.
mkdir -p "$install_dir/.jev-update"
printf '{\n  "installer": "install.sh",\n  "version": "%s",\n  "target": "%s"\n}\n' "$version" "$target" \
  > "$install_dir/.jev-update/receipt.json.tmp"
mv -f "$install_dir/.jev-update/receipt.json.tmp" "$install_dir/.jev-update/receipt.json"

say "Installed jev $version ($target) to $install_dir/jev"
if [ -n "$signature" ]; then
  say "Verified: the SHA-256 checksum, and $signature."
else
  say "Verified: the SHA-256 checksum against SHA256SUMS."
  say "Not verified: the signature, because minisign is not installed. The checksum proves the"
  say "download is intact, not who made it; to check that too, install minisign and run this again,"
  say "or follow https://github.com/shaharia-lab/jev-cli/blob/main/SECURITY.md#verifying-a-release"
fi

# ── PATH ───────────────────────────────────────────────────────────────────────────────────────
case $install_dir in
  "${HOME-}"/*) shown_dir="\$HOME/${install_dir#"$HOME"/}" ;;
  *) shown_dir=$install_dir ;;
esac
case ":${PATH-}:" in
  *":$install_dir:"*)
    if [ -n "$previous" ] && [ "$previous" != "$install_dir/jev" ]; then
      say "warning: another jev at $previous comes first on PATH"
    fi
    say "Run 'jev --help' to get started."
    exit 0
    ;;
esac

path_line="export PATH=\"$shown_dir:\$PATH\""
profile=
if [ "$no_modify_path" = 0 ] && [ -n "${HOME-}" ]; then
  case $install_dir in
    *[\"\\\$\`]*) ;; # would need quoting in the profile; only give guidance
    *)
      case $(basename "${SHELL-sh}") in
        zsh) profile=${ZDOTDIR:-$HOME}/.zshrc ;;
        bash) if [ "$os" = Darwin ]; then profile=$HOME/.bash_profile; else profile=$HOME/.bashrc; fi ;;
        fish)
          profile=${XDG_CONFIG_HOME:-$HOME/.config}/fish/conf.d/jev.fish
          path_line="fish_add_path --global \"$install_dir\""
          ;;
        *) profile=$HOME/.profile ;;
      esac
      ;;
  esac
fi
if [ -n "$profile" ]; then
  if [ -f "$profile" ] && grep -qxF "$path_line" "$profile"; then
    say "$profile already adds $shown_dir to PATH."
  # jev is installed by now: a profile that cannot be written is a warning, not a failure.
  elif { mkdir -p "$(dirname "$profile")" &&
    printf '\n# Added by the jev installer\n%s\n' "$path_line" >> "$profile"; } 2> /dev/null; then
    say "Added $shown_dir to PATH in $profile."
  else
    say "warning: could not write $profile"
    profile=
  fi
fi
if [ -n "$profile" ]; then
  say "Open a new terminal, or run this in the current one, to use jev:"
else
  say "$shown_dir is not on your PATH. Add it by adding this line to your shell profile:"
fi
say "  $path_line"
say "Then run 'jev --help' to get started."
