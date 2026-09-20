# Releasing

Nobody edits a version number or `CHANGELOG.md` by hand. A release is cut by
[release-please](https://github.com/googleapis/release-please) from the Conventional Commit
titles on `main`, and every artefact is signed and verified before it is published.

The step-by-step procedure, including how to steer a version and how to resume a failed run, is
in [CONTRIBUTING.md](../../CONTRIBUTING.md#releases). This page is the shape of the pipeline and
the reasoning behind it.

## The path a release takes

```text
merge to main
  -> release-please updates a release pull request (next version + changelog)
  -> merging it tags vX.Y.Z and drafts a GitHub Release
  -> the tag starts release.yml:
       plan -> build (6 native runners) -> package (+ SBOM)
            -> publish (sign, upload to the draft)   [environment: release, owner approves]
            -> attest -> verify -> finalize -> stable
  -> stable only: Homebrew tap, then crates.io
```

A pre-release (`0.2.0-rc.1`) runs the same pipeline and is published as a GitHub pre-release that
never becomes "latest". Everything that must not see one, the Homebrew formula and crates.io,
hangs off the `stable` job, which a pre-release skips. Dispatching the workflow on a branch is a
dry run: everything is built, checked and packed, and nothing is published.

## Why it is staged

Each stage exists to make one class of mistake impossible.

- **Build on native runners**, six targets, each binary checked for size, version and static
  linking. No cross-compilation surprises.
- **Sign in one job only.** The signing key exists in the `release` environment and nowhere else,
  and that environment admits `v*` tags and requires the owner's approval. Every asset and
  `SHA256SUMS` is signed with minisign, and each signature's trusted comment names the file and
  the version, so a signature cannot be replayed onto another release.
- **Attest provenance** for every asset, so anyone can trace a binary back to the workflow and
  the commit that produced it.
- **Verify what was uploaded**, not what was built: the assets are downloaded again and their
  checksums, signatures and attestations are checked with the standard tools before the release
  is published.
- **Publish downstream last.** The Homebrew tap and crates.io only ever see a release that
  passed all of the above.

## Secrets live in environments, not in the repository

A repository secret is readable by any workflow run on any branch, so credentials that can
publish live in GitHub environments with a deployment rule instead:

| Environment | Holds | Admits |
| --- | --- | --- |
| `release` | The minisign signing key and the crates.io token | `v*` tags, with the owner's approval |
| `publish` | The Homebrew tap App credentials | `v*` tags |
| `release-please` | The release bot App credentials | `main` |
| `live-smoke` | The API key for the nightly live test | `main` |

## Verification anyone can repeat

Every release carries `SHA256SUMS`, a minisign signature per file, a CycloneDX SBOM and build
provenance attestations. The public keys are committed in `crates/jev-cli/keys/` and listed in
[SECURITY.md](../../SECURITY.md), which also has the exact verification commands and the key
rotation procedure. `scripts/release/self-test.sh`, part of `make check`, drives signing and
verification with a throwaway key, tampered assets included, so the release path is exercised on
every pull request rather than once a release.

## The install scripts

`install.sh` and `install.ps1` are served to users straight from `main`, so a change to them
ships the moment it merges, without a release. They embed the release public keys and always
check the checksum. `crates/jev-cli/tests/install_script.rs` runs them on every operating system
against signed fake releases on a local server, and a lint script runs ShellCheck and
PSScriptAnalyzer over them.

## The moving parts

`release-please.yml` tags and drafts the release as the `jev-release-bot` App, because a tag or a
pull request made with `GITHUB_TOKEN` triggers no workflow. The tag starts `release.yml`, which is
staged: plan, build on six native runners, package with a CycloneDX SBOM, publish (sign and
upload to the draft, in the `release` environment), attest, verify, finalize, `stable`. Any other
ref is a dry run. `scripts/release/verify-assets.sh` checks the archive layout, the full asset
list and the signatures; change it together with the build matrix.

**Homebrew.** The formulas come from `scripts/release/homebrew-formula.sh`, with checksums taken
from the verified `SHA256SUMS`, and reach `shaharia-lab/homebrew-tap` through
`homebrew-publish.sh` (contents API, idempotent, and it never moves `jev.rb` back to an older
release). `homebrew-check` runs on every release run of a stable version, dry runs included:
`brew style`, `brew audit --strict`, then `brew install` and `brew test` from that run's
archives. The formula keeps the binary in the keg (`Cellar/jev/<version>/bin/jev`, beside
Homebrew's `INSTALL_RECEIPT.json`), which is how `jev update` recognises a managed install.

**crates.io** gets `jev-client` first, then `jev-cli` once the index has the library, from
`scripts/release/crates-publish.sh` (the `crates` job: `stable`, `release` environment,
`CARGO_REGISTRY_TOKEN`). A version already in the index is skipped, because a publish can never
be undone, so re-running after a partial publish is safe. `crates/jev-cli` packages only what
builds `jev` (its `include` list), and `[package.metadata.binstall]` names the release archives
and the primary signing key. Every pull request runs `cargo publish --workspace --dry-run`
(`make package`).

**Signing.** Every asset and `SHA256SUMS` is signed with minisign by the one job that can reach
the signing key. The public keys are `crates/jev-cli/keys/release-primary.pub` (which signs) and
`release-next.pub` (for rotation); the updater trusts both and only those. Each signature's
trusted comment is exactly `file:<name>\tversion:<version>`, and verification checks it.
`SECURITY.md` lists the keys, and `scripts/release/self-test.sh` fails when the two disagree.

**Install scripts.** `install.sh` (POSIX sh) and `install.ps1` (PowerShell 5.1 and 7) live at the
repository root and are served to users from `main`, so a change to them ships when it merges,
without a release. They embed the release keys, always check the checksum and check the signature
when `minisign` is on the `PATH`, and write `<dir>/.jev-update/receipt.json` (`installer`,
`version`, `target`), which is how `jev update` recognises a self-managed install. Their release
constants sit in one block, one assignment per line, because
`crates/jev-cli/tests/install_script.rs` replaces only that block to run them against a local
server, and checks that the shipped values are GitHub over HTTPS and the committed keys.
`install.ps1` must stay ASCII.

## Cutting a release

There is a skill for this: `.claude/skills/release/SKILL.md` walks an agent (or a person)
through readiness, steering the version, approving the environment gate and verifying the
published release. The underlying procedure is in
[CONTRIBUTING.md](../../CONTRIBUTING.md#releases).
