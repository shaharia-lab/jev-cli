# Installation

`jev` is a single binary. There is nothing to configure at install time and nothing runs in the
background except an optional once-a-day update check that you can turn off.

Supported platforms: Linux (x86-64, arm64, statically linked), macOS (Intel, Apple silicon) and
Windows (x86-64, arm64).

## Install script (Linux and macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/shaharia-lab/jev-cli/main/install.sh | sh
```

It installs the latest stable release into `~/.local/bin`, adds that directory to your `PATH` in
your shell profile if it is not there already, and never uses `sudo`.

| Option | What it does |
| --- | --- |
| `--version 0.1.0` | Install one particular release, pre-releases included |
| `--install-dir ~/bin` | Install somewhere else |
| `--no-modify-path` | Leave your shell profile alone and print what to add |
| `--require-signature` | Refuse to install unless the minisign signature is verified |

Pass them after `sh -s --`, for example:

```bash
curl -fsSL https://raw.githubusercontent.com/shaharia-lab/jev-cli/main/install.sh | sh -s -- --version 0.1.0
```

Every option also has an environment variable (`JEV_INSTALL_VERSION`, `JEV_INSTALL_DIR`,
`JEV_INSTALL_NO_MODIFY_PATH`, `JEV_INSTALL_REQUIRE_SIGNATURE`), and `sh install.sh --help`
explains all of them.

## Install script (Windows)

```powershell
irm https://raw.githubusercontent.com/shaharia-lab/jev-cli/main/install.ps1 | iex
```

It installs into `%LOCALAPPDATA%\Programs\jev` and adds that to your user `PATH`. It works in
Windows PowerShell 5.1 and in PowerShell 7, and it never asks for elevation. To pick a version,
set the environment variable first:

```powershell
$env:JEV_INSTALL_VERSION = '0.1.0'; irm https://raw.githubusercontent.com/shaharia-lab/jev-cli/main/install.ps1 | iex
```

## Homebrew (macOS and Linux)

```bash
brew install shaharia-lab/tap/jev
```

Homebrew also installs the man pages and the shell completions, so `man jev` and tab completion
work straight away. Update it with `brew upgrade jev`.

## Cargo (any platform Rust supports)

```bash
cargo binstall jev-cli          # downloads the release archive and checks its signature
cargo install jev-cli --locked  # builds from source
```

Both record the install in cargo's own `.crates.toml`, so `jev update` will tell you to use cargo
rather than replacing the binary itself. The library behind the CLI is published separately as
[`jev-client`](https://crates.io/crates/jev-client).

## Download a binary by hand

Every release on the [releases page](https://github.com/shaharia-lab/jev-cli/releases) carries an
archive per platform, a `SHA256SUMS` file, a minisign signature for each file and a CycloneDX
SBOM. Unpack the archive and put `jev` somewhere on your `PATH`.

## What the install scripts verify

Before anything is written to disk, both scripts:

1. download the release archive and `SHA256SUMS` over HTTPS,
2. check the archive's SHA-256 against `SHA256SUMS`, always,
3. check the minisign signatures of both files against the release keys built into the script,
   when [minisign](https://jedisct1.github.io/minisign/) is on your `PATH`,
4. run the new binary once and confirm it reports the version that was asked for.

They say which checks ran. Use `--require-signature` to turn a missing `minisign` into a failure
instead of a warning. To verify a download yourself, follow
[Verifying a release](../../SECURITY.md#verifying-a-release), which lists the public keys and the
exact commands.

## Check that it worked

```bash
jev version
jev --help
```

`jev version` also reports how `jev` was installed and whether it will update itself.

## Shell completions and man pages

Homebrew sets both up for you. Otherwise `jev completion <shell>` prints a completion script for
the exact `jev` you are running. Enable it once, and run it again after an upgrade.

| Shell | Enable it |
| --- | --- |
| Bash | add `source <(jev completion bash)` to `~/.bashrc` |
| Zsh | add `source <(jev completion zsh)` to `~/.zshrc`, after `compinit` |
| Fish | `jev completion fish > ~/.config/fish/completions/jev.fish` |
| PowerShell | add `jev completion powershell \| Out-String \| Invoke-Expression` to `$PROFILE` |

## Uninstall

Delete the binary. A script install keeps everything in one directory, so removing
`~/.local/bin/jev` and `~/.local/bin/.jev-update/` is enough. With Homebrew, `brew uninstall jev`;
with cargo, `cargo uninstall jev-cli`.

Your settings and stored key live in the configuration directory that `jev config path` prints.
Delete that directory too if you want no trace left.

## Next

- [Quick start](quick-start.md): give `jev` a key and ask your first question.
- [Keeping jev up to date](updates.md): how updates work and how to turn them off.
