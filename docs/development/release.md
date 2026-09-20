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
