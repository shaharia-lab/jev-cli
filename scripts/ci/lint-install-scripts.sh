#!/usr/bin/env bash
# Lints the install scripts: install.sh with ShellCheck (as POSIX sh, which is what `curl | sh`
# runs), install.ps1 with PSScriptAnalyzer (every warning and error).
#
#   scripts/ci/lint-install-scripts.sh
#
# A missing linter is skipped with a warning, except in CI (CI=true), where both must run.
# Their behaviour is tested by crates/jev-cli/tests/install_script.rs.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
failed=0
missing() {
  if [ "${CI-}" = true ]; then
    echo "error: $1 is not installed" >&2
    failed=1
  else
    echo "warning: $1 is not installed; skipping it" >&2
  fi
}

if command -v shellcheck > /dev/null; then
  shellcheck --shell=sh --severity=style "$root/install.sh" || failed=1
  echo "shellcheck: install.sh checked"
else
  missing shellcheck
fi

if command -v pwsh > /dev/null; then
  # PSScriptAnalyzer ships with the GitHub runner images; elsewhere: Install-Module PSScriptAnalyzer.
  if ! INSTALL_PS1="$root/install.ps1" pwsh -NoProfile -NonInteractive -Command '
    $ErrorActionPreference = "Stop"
    if (-not (Get-Module -ListAvailable PSScriptAnalyzer)) { Write-Error "PSScriptAnalyzer is not installed" }
    $findings = Invoke-ScriptAnalyzer -Path $env:INSTALL_PS1 -Severity Warning, Error
    $findings | Format-Table -AutoSize -Property Severity, Line, RuleName, Message | Out-String -Width 200 | Write-Host
    if ($findings) { exit 1 }
    Write-Host "PSScriptAnalyzer: install.ps1 checked"
  '; then
    failed=1
  fi
else
  missing pwsh
fi

exit "$failed"
