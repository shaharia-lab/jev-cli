<#
.SYNOPSIS
Installs jev, an unofficial command-line tool for TypeSafe AI's Jev model, on Windows.

.DESCRIPTION
Downloads the archive for this machine from the project's GitHub Releases, checks its SHA-256
against SHA256SUMS and, when minisign is on PATH, the minisign signatures of both against the jev
release keys (each signature must be made for that file of that version), runs the new jev.exe
once, and installs it into a directory you own. It never asks for elevation. Nothing is installed
unless every check passes.

The directory gets jev.exe and .jev-update\receipt.json, which marks the install as self-managed so
that `jev update` can update it. The directory is added to your user PATH unless -NoModifyPath is
given. Messages go to stderr; nothing is printed on stdout.

When run as a file, the exit status is 0 when jev was installed, 1 when it failed (nothing was
installed) and 2 for a usage error. Piped into Invoke-Expression, it reports an error and returns
without closing your session.

Linux and macOS: install.sh. Anything else: cargo install jev-cli --locked.

.PARAMETER Version
The version to install, pre-releases (0.1.0-rc.1) included. Default: the latest stable release.
Also read from the JEV_INSTALL_VERSION environment variable.

.PARAMETER InstallDir
Where to put jev.exe. Default: %LOCALAPPDATA%\Programs\jev. Also read from JEV_INSTALL_DIR.

.PARAMETER NoModifyPath
Do not add the directory to the user PATH; only say how. Also set by JEV_INSTALL_NO_MODIFY_PATH=1.

.PARAMETER RequireSignature
Fail unless the signature is verified, which needs minisign on PATH. Also set by
JEV_INSTALL_REQUIRE_SIGNATURE=1.

.EXAMPLE
irm https://raw.githubusercontent.com/shaharia-lab/jev-cli/main/install.ps1 | iex

.EXAMPLE
$env:JEV_INSTALL_VERSION = '0.1.0-rc.1'; irm https://raw.githubusercontent.com/shaharia-lab/jev-cli/main/install.ps1 | iex

.EXAMPLE
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/shaharia-lab/jev-cli/main/install.ps1))) -InstallDir "$HOME\bin" -NoModifyPath
#>
[CmdletBinding()]
param(
    [string]$Version = $env:JEV_INSTALL_VERSION,
    [string]$InstallDir = $env:JEV_INSTALL_DIR,
    [switch]$NoModifyPath,
    [switch]$RequireSignature
)

# Everything runs inside functions, so that piping this script into Invoke-Expression changes no
# preference or variable of the caller's session (except PATH, when asked to).

function Write-Message([string]$Text) {
    [Console]::Error.WriteLine($Text)
}

# A failure that has been explained: its message is printed as is, with the exit status it carries.
function Exit-Install([string]$Text, [int]$ExitCode = 1) {
    $failure = [Exception]::new($Text)
    $failure.Data['JevInstallExitCode'] = $ExitCode
    throw $failure
}

function Test-Explained($ErrorRecord) {
    return $ErrorRecord.Exception.Data.Contains('JevInstallExitCode')
}

# Runs a program and returns what it printed on stdout; its exit status is in $LASTEXITCODE.
# Windows PowerShell turns a program's stderr into errors, which 'Stop' would make fatal.
function Invoke-Program([string]$Program, [string[]]$Arguments) {
    $ErrorActionPreference = 'Continue'
    & $Program @Arguments 2> $null
}

function Test-Flag([string]$Value) {
    return -not [string]::IsNullOrEmpty($Value) -and $Value -ne '0' -and $Value -ne 'false'
}

function Install-Jev {
    param([string]$Version, [string]$InstallDir, [bool]$NoModifyPath, [bool]$RequireSignature)

    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'

    # -- Where releases come from, and which keys may sign them ----------------------------------
    # One assignment per line: crates/jev-cli/tests/install_script.rs replaces these to install
    # from a local server with a key of its own.
    $RepositoryUrl = 'https://github.com/shaharia-lab/jev-cli'
    $ApiUrl = 'https://api.github.com/repos/shaharia-lab/jev-cli'
    $AllowHttp = $false
    $Minisign = 'minisign'
    # The release public keys: primary, then next (SECURITY.md, crates/jev-cli/keys). A release
    # signed by either is accepted, as `jev update` accepts it.
    $PublicKeys = @('RWS4h5k7TFvpPJt8wAoJCpoWgejSO8eVKRs+CqEdFIQAP2E0QwyyDDcU', 'RWQIs3BoLNvYo0e6bXIBfp7hoYpwkQSJuAWcSD7itrj0ULCXWULmDMx4')

    $RawUrl = 'https://raw.githubusercontent.com/shaharia-lab/jev-cli/main'

    # -- Options ---------------------------------------------------------------------------------
    if ($Version.StartsWith('v')) { $Version = $Version.Substring(1) }
    if ($Version -and $Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$') {
        Exit-Install "'$Version' is not a version such as 0.1.0 or 0.1.0-rc.1" 2
    }
    if (-not $InstallDir) {
        if (-not $env:LOCALAPPDATA) { Exit-Install 'LOCALAPPDATA is not set; choose a directory with -InstallDir' 2 }
        $InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\jev'
    }
    $InstallDir = [IO.Path]::GetFullPath($InstallDir)

    # -- Platform --------------------------------------------------------------------------------
    if ($PSVersionTable.PSEdition -eq 'Core' -and -not $IsWindows) {
        Exit-Install "this script is for Windows. On Linux and macOS, run:`n  curl -fsSL $RawUrl/install.sh | sh"
    }
    $architecture = $null
    try {
        $architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    } catch {
        $architecture = $null
    }
    if (-not $architecture) {
        # A 32-bit PowerShell on 64-bit Windows sees its own architecture in PROCESSOR_ARCHITECTURE.
        $architecture = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
    }
    switch ($architecture) {
        { $_ -in 'X64', 'AMD64' } { $target = 'x86_64-pc-windows-msvc' }
        { $_ -in 'Arm64', 'ARM64' } { $target = 'aarch64-pc-windows-msvc' }
        default {
            Exit-Install "there is no prebuilt jev for the $architecture architecture. Build it from source with Rust instead:`n  cargo install jev-cli --locked"
        }
    }

    # -- Downloads (HTTPS only) ------------------------------------------------------------------
    # Windows PowerShell 5.1 does not offer TLS 1.2 by default.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    function Get-Download([string]$Url, [string]$File) {
        if (-not ($Url.StartsWith('https://') -or ($AllowHttp -and $Url.StartsWith('http://')))) {
            Exit-Install "refusing $Url`: only https:// is allowed"
        }
        Invoke-WebRequest -Uri $Url -OutFile $File -UseBasicParsing -MaximumRedirection 5 -TimeoutSec 60
    }

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("jev-install-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    $staged = $null
    try {
        if (-not $Version) {
            try {
                Get-Download "$ApiUrl/releases/latest" (Join-Path $tmp 'latest.json')
                $tag = (Get-Content -Raw (Join-Path $tmp 'latest.json') | ConvertFrom-Json).tag_name
            } catch {
                if (Test-Explained $_) { throw }
                Exit-Install "could not find the latest stable release of jev ($($_.Exception.Message)).`nThere may be no stable release yet: install a pre-release with -Version <x.y.z-rc.N>`n(see $RepositoryUrl/releases)."
            }
            if ("$tag" -notmatch '^v?([0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?)$') {
                Exit-Install "could not read the latest version from $ApiUrl/releases/latest; pass -Version <x.y.z>"
            }
            $Version = $Matches[1]
        }

        $archive = "jev-$Version-$target.zip"
        $base = "$RepositoryUrl/releases/download/v$Version"
        Write-Message "Downloading jev $Version for $target"
        foreach ($file in @('SHA256SUMS', 'SHA256SUMS.minisig', $archive, "$archive.minisig")) {
            try {
                Get-Download "$base/$file" (Join-Path $tmp $file)
            } catch {
                if (Test-Explained $_) { throw }
                Exit-Install "could not download $base/$file ($($_.Exception.Message)).`nCheck that jev $Version exists and has a $target archive: $RepositoryUrl/releases"
            }
        }

        # -- Verification ------------------------------------------------------------------------
        $verifier = Get-Command $Minisign -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($verifier) {
            foreach ($name in @('SHA256SUMS', $archive)) {
                $expectedComment = "file:$name`tversion:$Version"
                $path = Join-Path $tmp $name
                $verified = $false
                foreach ($key in $PublicKeys) {
                    # -H: only prehashed signatures, which minisign makes. -Q prints the trusted
                    # comment, and only when the signature is valid.
                    $comment = Invoke-Program $verifier.Source @('-V', '-H', '-Q', '-P', $key, '-m', $path, '-x', "$path.minisig")
                    if ($LASTEXITCODE -eq 0) {
                        if ("$comment".Trim() -ne $expectedComment) {
                            Exit-Install "$name.minisig is a valid signature, but for another file or release (its trusted comment is '$comment'). Nothing was installed."
                        }
                        $verified = $true
                        break
                    }
                }
                if (-not $verified) {
                    Exit-Install "$name.minisig is not a valid signature of $name by a jev release key. Nothing was installed."
                }
            }
            $signature = 'the minisign signatures of the archive and SHA256SUMS by a jev release key'
        } elseif ($RequireSignature) {
            Exit-Install '-RequireSignature was given, but minisign is not installed (https://jedisct1.github.io/minisign/). Nothing was installed.'
        } else {
            $signature = $null
        }

        $listed = @(Get-Content (Join-Path $tmp 'SHA256SUMS') | ForEach-Object {
                if ($_ -match '^([0-9a-f]{64}) [ *](.+)$' -and $Matches[2] -eq $archive) { $Matches[1] }
            })
        if ($listed.Count -ne 1) {
            Exit-Install "SHA256SUMS does not list $archive exactly once. Nothing was installed."
        }
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $tmp $archive)).Hash.ToLowerInvariant()
        if ($actual -ne $listed[0]) {
            Exit-Install "the SHA-256 of $archive does not match SHA256SUMS: the download is corrupt or was altered. Nothing was installed."
        }

        # -- Install -----------------------------------------------------------------------------
        $extract = Join-Path $tmp 'extract'
        try {
            Expand-Archive -LiteralPath (Join-Path $tmp $archive) -DestinationPath $extract
        } catch {
            Exit-Install "$archive is not a readable zip archive: $($_.Exception.Message)"
        }
        $binary = Join-Path $extract "jev-$Version-$target\jev.exe"
        if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
            Exit-Install "$archive does not contain jev-$Version-$target\jev.exe"
        }

        $previous = Get-Command jev -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        try {
            New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
        } catch {
            Exit-Install "could not create $InstallDir; choose another directory with -InstallDir (this script never asks for elevation)"
        }
        $destination = Join-Path $InstallDir 'jev.exe'
        if (Test-Path -LiteralPath $destination -PathType Container) {
            Exit-Install "$destination is a directory"
        }

        # Staged in the install directory, so that the final move stays on one volume.
        $staged = Join-Path $InstallDir ".jev-install-$PID.exe"
        try {
            Copy-Item -LiteralPath $binary -Destination $staged -Force
        } catch {
            Exit-Install "$InstallDir is not writable by you; choose another directory with -InstallDir (this script never asks for elevation)"
        }
        $configDir = $env:JEV_CONFIG_DIR
        try {
            $env:JEV_CONFIG_DIR = Join-Path $tmp 'config'
            $reported = Invoke-Program $staged @('version', '--field', 'version')
            $status = $LASTEXITCODE
        } catch {
            $status = -1
        } finally {
            $env:JEV_CONFIG_DIR = $configDir
        }
        if ($status -ne 0) {
            Exit-Install 'the downloaded jev.exe does not run on this system. Nothing was installed.'
        }
        if ("$reported".Trim() -ne $Version) {
            Exit-Install "the downloaded jev.exe reports version '$reported', not $Version. Nothing was installed."
        }

        # A running jev.exe cannot be replaced or deleted, only renamed: it is moved into the
        # updater's trash, which `jev update` empties later, and put back if the move fails.
        $state = Join-Path $InstallDir '.jev-update'
        New-Item -ItemType Directory -Force -Path $state | Out-Null
        $aside = $null
        if (Test-Path -LiteralPath $destination) {
            $trash = Join-Path $state 'trash'
            New-Item -ItemType Directory -Force -Path $trash | Out-Null
            $aside = Join-Path $trash ("jev-" + [Guid]::NewGuid().ToString('N') + '.exe')
            Move-Item -LiteralPath $destination -Destination $aside
        }
        try {
            Move-Item -LiteralPath $staged -Destination $destination
        } catch {
            if ($aside) { Move-Item -LiteralPath $aside -Destination $destination }
            Exit-Install "could not install $destination`: $($_.Exception.Message)"
        }
        $staged = $null
        if ($aside) { Remove-Item -LiteralPath $aside -Force -ErrorAction SilentlyContinue }

        # The receipt tells `jev update` that this install is self-managed, so it may replace the
        # binary. UTF-8 without a byte order mark, which Windows PowerShell's Set-Content adds.
        $receipt = "{`n  `"installer`": `"install.ps1`",`n  `"version`": `"$Version`",`n  `"target`": `"$target`"`n}`n"
        [IO.File]::WriteAllText((Join-Path $state 'receipt.json.tmp'), $receipt, [Text.UTF8Encoding]::new($false))
        Move-Item -LiteralPath (Join-Path $state 'receipt.json.tmp') -Destination (Join-Path $state 'receipt.json') -Force

        Write-Message "Installed jev $Version ($target) to $destination"
        if ($signature) {
            Write-Message "Verified: the SHA-256 checksum, and $signature."
        } else {
            Write-Message 'Verified: the SHA-256 checksum against SHA256SUMS.'
            Write-Message 'Not verified: the signature, because minisign is not installed. The checksum proves the'
            Write-Message 'download is intact, not who made it; to check that too, install minisign and run this again,'
            Write-Message 'or follow https://github.com/shaharia-lab/jev-cli/blob/main/SECURITY.md#verifying-a-release'
        }

        # -- PATH --------------------------------------------------------------------------------
        if ($previous -and $previous.Source -ne $destination) {
            Write-Message "warning: another jev at $($previous.Source) comes first on PATH"
        }
        $normalize = { param($entry) $entry.Trim().TrimEnd('\') }
        $wanted = & $normalize $InstallDir
        $addPath = "[Environment]::SetEnvironmentVariable('Path', [Environment]::GetEnvironmentVariable('Path', 'User') + ';$InstallDir', 'User')"
        if (@($env:Path -split ';' | Where-Object { $_ } | ForEach-Object { & $normalize $_ }) -contains $wanted) {
            Write-Message "Run 'jev --help' to get started."
            return
        }
        if ($NoModifyPath) {
            Write-Message "$InstallDir is not on your PATH. To add it for your user, run:"
            Write-Message "  $addPath"
            Write-Message "Then open a new terminal and run 'jev --help' to get started."
            return
        }
        # jev is installed by now: a PATH that cannot be changed is a warning, not a failure.
        try {
            # The user PATH as stored, with %VARIABLES% unexpanded, so that writing it back keeps them.
            $userPath = [string](Get-Item -Path 'HKCU:\Environment').GetValue('Path', '', 'DoNotExpandEnvironmentNames')
            $inUserPath = @($userPath -split ';' | Where-Object { $_ } | ForEach-Object {
                    & $normalize ([Environment]::ExpandEnvironmentVariables($_))
                }) -contains $wanted
            if (-not $inUserPath) {
                $newPath = if ($userPath) { $userPath.TrimEnd(';') + ";$InstallDir" } else { $InstallDir }
                New-ItemProperty -Path 'HKCU:\Environment' -Name 'Path' -Value $newPath -PropertyType ExpandString -Force | Out-Null
                # Setting a user variable through .NET tells running programs, Explorer included,
                # that the environment changed; the registry write above does not.
                [Environment]::SetEnvironmentVariable('JEV_INSTALL_PATH_CHANGED', '1', 'User')
                [Environment]::SetEnvironmentVariable('JEV_INSTALL_PATH_CHANGED', $null, 'User')
                Write-Message "Added $InstallDir to your user PATH."
            }
            $env:Path = "$env:Path;$InstallDir"
            Write-Message "Open a new terminal and run 'jev --help' to get started."
        } catch {
            Write-Message "warning: jev is installed, but $InstallDir could not be added to your PATH ($($_.Exception.Message)). To add it, run:"
            Write-Message "  $addPath"
        }
    } finally {
        if ($staged) { Remove-Item -LiteralPath $staged -Force -ErrorAction SilentlyContinue }
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

$code = 0
try {
    Install-Jev -Version $Version -InstallDir $InstallDir `
        -NoModifyPath ($NoModifyPath.IsPresent -or (Test-Flag $env:JEV_INSTALL_NO_MODIFY_PATH)) `
        -RequireSignature ($RequireSignature.IsPresent -or (Test-Flag $env:JEV_INSTALL_REQUIRE_SIGNATURE))
} catch {
    Write-Message "error: $($_.Exception.Message)"
    $code = if (Test-Explained $_) { $_.Exception.Data['JevInstallExitCode'] } else { 1 }
}
# Run as a file, the exit status carries the outcome. Piped into Invoke-Expression, `exit` would
# close the caller's session, so the error message has to do.
if ($MyInvocation.MyCommand.Path) {
    exit $code
}
