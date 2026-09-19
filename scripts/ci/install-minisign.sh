#!/usr/bin/env bash
# Installs the minisign CLI on a CI runner, so that the tests of install.sh and install.ps1
# (crates/jev-cli/tests/install_script.rs) check signatures on every OS:
#
# - Linux: the distribution package.
# - macOS: Homebrew.
# - Windows: the author's release archive, pinned by SHA-256 (verified against the author's
#   minisign key when the pin was taken), added to the job's PATH through GITHUB_PATH.
#
# To upgrade the Windows build: bump the version, download the new archive and its .minisig, check
# it with `minisign -Vm <zip> -P RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3`, and
# update the checksum with the output of `sha256sum`.
set -euo pipefail

MINISIGN_VERSION="0.12"
MINISIGN_WIN64_SHA256="37b600344e20c19314b2e82813db2bfdcc408b77b876f7727889dbd46d539479"

case $(uname -s) in
  Linux)
    sudo apt-get update -qq
    sudo apt-get install -y -qq minisign
    ;;
  Darwin)
    HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALL_CLEANUP=1 brew install --quiet minisign
    ;;
  MINGW* | MSYS* | CYGWIN*)
    : "${GITHUB_PATH:?GITHUB_PATH is not set: this branch is for GitHub Actions only}"
    case ${PROCESSOR_ARCHITECTURE-} in
      ARM64) arch=aarch64 ;;
      *) arch=x86_64 ;;
    esac
    dir="${RUNNER_TEMP:-$(mktemp -d)}/minisign"
    mkdir -p "$dir"
    zip="$dir/minisign.zip"
    curl --proto '=https' --proto-redir '=https' --tlsv1.2 --fail --silent --show-error --location --retry 3 \
      "https://github.com/jedisct1/minisign/releases/download/$MINISIGN_VERSION/minisign-$MINISIGN_VERSION-win64.zip" \
      -o "$zip"
    echo "$MINISIGN_WIN64_SHA256  $zip" | sha256sum --check --quiet
    unzip -q -o "$zip" -d "$dir"
    cygpath -w "$dir/minisign-win64/$arch" >> "$GITHUB_PATH"
    export PATH="$dir/minisign-win64/$arch:$PATH"
    ;;
  *)
    echo "error: no way to install minisign on $(uname -s)" >&2
    exit 1
    ;;
esac

minisign -v
