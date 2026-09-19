#!/usr/bin/env bash
# Decides what a release workflow run does, from the git ref it runs on, and prints it as
# `key=value` lines for $GITHUB_OUTPUT:
#
#   scripts/release/plan.sh <git-ref>
#
#   version     the workspace version in Cargo.toml, which the binaries report
#   publish     true only on a tag `v<version>`; any other ref is a dry run that publishes nothing
#   tag         the tag to publish, empty for a dry run
#   prerelease  true when the version has a pre-release part (0.2.0-rc.1): published as a GitHub
#               pre-release, never marked latest, and never sent to crates.io or the Homebrew tap
#
# A tag that starts with `v` but is not `v<version>` fails, so a release can never ship binaries
# whose version disagrees with their tag.
set -euo pipefail

if [ $# -ne 1 ]; then
  echo "usage: $0 <git-ref>" >&2
  exit 2
fi
ref=$1
root=$(cd "$(dirname "$0")/../.." && pwd)

version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' "$root/Cargo.toml")
if ! grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$' <<< "$version"; then
  echo "error: [workspace.package] version in Cargo.toml is not a SemVer version: '$version'" >&2
  exit 1
fi

publish=false tag=""
case $ref in
  refs/tags/v*)
    tag=${ref#refs/tags/}
    if [ "$tag" != "v$version" ]; then
      echo "error: tag $tag does not match the version in Cargo.toml ($version)." >&2
      echo "Tag the commit whose Cargo.toml says ${tag#v}, or release through the release-please pull request." >&2
      exit 1
    fi
    publish=true
    ;;
esac

prerelease=false
case $version in *-*) prerelease=true ;; esac

echo "version=$version"
echo "publish=$publish"
echo "tag=$tag"
echo "prerelease=$prerelease"
