# Keeping jev up to date

How `jev` updates itself depends on how it was installed. `jev version` says which case you are
in, and why.

| Installed with | Updated by |
| --- | --- |
| The install script | `jev update`, and automatically in the background |
| Homebrew | `brew upgrade jev` |
| `cargo install` or `cargo binstall` | `cargo install jev-cli --locked` |
| A binary you unpacked yourself | Download the next one, or use the install script |

## Updating by hand

```bash
jev update --check      # exit 20 when a newer release exists, and nothing changes
jev update              # move to the latest release now
jev update --rollback   # undo the last update
```

`jev update` keeps the binary it replaced, which is what `--rollback` puts back. After writing
the new binary it runs it once and puts the old one back if that fails, so a broken download
cannot leave you without a working `jev`.

A package-managed install is not replaced. `jev update` recognises it and prints the command for
that package manager instead.

## Automatic updates

A `jev` installed by the install script keeps itself current:

- At most once a day, **after** a command has finished, a detached process asks GitHub Releases
  whether there is something newer and stages it.
- The **next** command swaps it in and says so in one line on stderr.
- A command's output, its exit code and its timing never change. Nothing ever waits for the
  check.
- It never downgrades on its own. Only `jev update --version <x.y.z>` can move backwards.

Every download is verified before anything is replaced: the minisign signature of `SHA256SUMS`
and of the archive against the two release keys compiled into the binary, that each signature
was made for that file of that version, and then the archive's SHA-256.

It is already off when `jev` came from a package manager, when `CI=true`, when `jev` cannot write
to its own directory, and when there is no install receipt (a development build, for example).

## Turning it off, or pinning a version

```bash
jev config set update.auto false        # on this machine, for good
export JEV_AUTO_UPDATE=false            # for one environment or one script
jev config set update.pin_version 0.1.0 # stay on one version
jev config set update.channel prerelease # follow pre-releases as well
```

Where the version of `jev` must not change under a workflow, set `JEV_AUTO_UPDATE=false`. With
`CI=true`, which most CI systems set, it is already off.

## What it contacts

Only GitHub Releases of `shaharia-lab/jev-cli`, and only for this. There is no telemetry, no
version ping and no analytics of any kind. The only other host `jev` ever contacts is the
TypeSafe base URL you configured, when you run a command that evaluates something.

## Checking a release yourself

Every asset ships with a SHA-256 in `SHA256SUMS`, a minisign signature and a build provenance
attestation. [Verifying a release](../../SECURITY.md#verifying-a-release) lists the public keys
and the exact commands.
