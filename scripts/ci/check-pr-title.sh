#!/usr/bin/env bash
# A pull request title must be a Conventional Commit header, because squash merging makes it the
# commit on main, and release-please reads those commits to choose the next version and write
# CHANGELOG.md.
#
#   scripts/ci/check-pr-title.sh "<title>"   # checks one title
#   scripts/ci/check-pr-title.sh --self-test # checks the rule against known good and bad titles
set -euo pipefail

TYPES='feat|fix|perf|refactor|docs|test|build|ci|chore|style|revert'
PATTERN="^($TYPES)(\([a-z0-9][a-z0-9._/-]*\))?!?: [^ ](.*[^ ])?$"

valid() {
  grep -qE "$PATTERN" <<< "$1"
}

if [ "${1-}" = --self-test ]; then
  failed=0
  for title in "feat: add jev completion" "fix(client): retry on 529" "feat(cli)!: rename --out" \
    "chore(main): release 0.1.0" "docs: x" "ci(release): build six targets"; do
    valid "$title" || { echo "error: rejected valid title: $title" >&2; failed=1; }
  done
  for title in "Add jev completion" "feat:add completion" "feat add completion" "Feat: add" \
    "feature: add completion" "feat(): empty scope" "feat(CLI): upper-case scope" "feat: " \
    "fix: trailing space " "wip: something" ""; do
    if valid "$title"; then
      echo "error: accepted invalid title: '$title'" >&2
      failed=1
    fi
  done
  [ "$failed" -eq 0 ] && echo "PR title rule: self-test passed"
  exit "$failed"
fi

if [ $# -ne 1 ]; then
  echo "usage: $0 <title> | --self-test" >&2
  exit 2
fi
if ! valid "$1"; then
  cat >&2 << EOF
error: the pull request title is not a Conventional Commit header: '$1'
Use '<type>[(scope)][!]: <description>', where <type> is one of:
  ${TYPES//|/, }
For example 'feat(cli): add jev completion' or 'fix: retry on 529'. A '!' marks a breaking change.
EOF
  exit 1
fi
echo "PR title is a Conventional Commit header"
