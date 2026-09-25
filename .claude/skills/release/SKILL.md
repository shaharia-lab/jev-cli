---
name: release
description: >
  Cut and publish a release of jev-cli end to end: check that main is releasable, steer the
  version, merge the release-please pull request, watch the staged release workflow, approve the
  `release` environment gate, and verify the published release (assets, minisign signatures,
  build provenance, the Homebrew tap, crates.io and a real install). Use when asked to make,
  cut, publish or verify a jev-cli release, including a pre-release.
license: MIT OR Apache-2.0
---

# Releasing jev-cli

This is the running order. How the pipeline works and why it is staged is in
[`docs/development/release.md`](../../../docs/development/release.md); the procedure in prose is
[`CONTRIBUTING.md`](../../../CONTRIBUTING.md#releases); the keys and verification commands are in
[`SECURITY.md`](../../../SECURITY.md#verifying-a-release). Read the page, do not restate it.

Set `V` to the version once (`V=0.2.0`) and reuse it. Requires `gh` authenticated with write
access, and `minisign` for the manual check.

## Rules for this job

- **Never** edit a version or `CHANGELOG.md` by hand: release-please owns both.
- **Never** print, fetch or echo a secret. The signing key exists only inside the `release`
  environment; nothing here needs it.
- **Never** approve a deployment for a run you have not looked at, and never re-tag a version.
  Republishing is done by re-running jobs, not by moving a tag.
- Only the repository owner may approve the `release` environment. If you are an agent, ask
  unless the owner has already authorised you for this release.

## 1. Is `main` releasable?

```bash
git switch main && git pull --ff-only
make check                               # every CI gate, foreground
gh run list --branch main --limit 5      # CI green on the head commit
gh pr list --state open --label bug      # nothing that should block a release
```

Also confirm the live smoke test has not filed a tracking issue about the API changing:
`gh issue list --search "live smoke"`.

## 2. Decide the version

release-please proposes one from the Conventional Commit titles since the last release: `feat`
bumps the minor, `fix` the patch, and before 1.0 a breaking change bumps the minor. It **never**
promotes a pre-release to a final version, so after `0.2.0-rc.1` it will propose `0.2.0-rc.2` or
`0.3.0-rc.1`, not `0.2.0`.

To choose the version yourself, merge any pull request with a `Release-As:` footer, then wait for
release-please to update its pull request:

```bash
gh pr merge <number> --squash --admin --body "Release-As: $V"
```

## 3. Merge the release pull request

```bash
gh pr list --state open --search "chore(main): release"
gh pr view <number>                      # version and changelog entry must be what you expect
gh pr merge <number> --squash --admin
```

Do not merge a release pull request whose proposed version is wrong; fix it with step 2 first.
Merging tags `v$V` and creates a **draft** GitHub Release, and the tag starts `release.yml`.

## 4. Watch the release run

```bash
gh run list --workflow release.yml --limit 3
gh run watch <run-id> --exit-status
```

Stages: plan, build (six native runners), package, publish, attest, verify, finalize, stable,
then Homebrew and crates.io for a stable version. A failed run is resumed with **Re-run failed
jobs**, or `gh workflow run release.yml --ref v$V` when it is too old to re-run. Publishing is
idempotent: a draft gets exactly the new assets, a published release is never changed, and
crates.io skips a version already in the index.

## 5. Approve the `release` environment

The run pauses before signing, because the signing key lives in that environment and crates.io
trusts only that environment to publish. The owner approves in the GitHub UI, or by API:

```bash
gh api /repos/shaharia-lab/jev-cli/actions/runs/<run-id>/pending_deployments
gh api --method POST /repos/shaharia-lab/jev-cli/actions/runs/<run-id>/pending_deployments \
  -F "environment_ids[]=<environment-id>" -f state=approved -f comment="release $V"
```

A stable release pauses once more if the crates job needs the same environment.

## 6. Verify the published release

Do this yourself; do not take the workflow's word for it.

```bash
gh release view "v$V" --json isDraft,isPrerelease,assets --jq \
  '{draft: .isDraft, prerelease: .isPrerelease, assets: (.assets | length)}'
```

Expect `draft: false`, one archive per platform plus the schemas, the SBOM, `SHA256SUMS` and a
`.minisig` beside every file. Then, in a scratch directory, follow
[`SECURITY.md`](../../../SECURITY.md#verifying-a-release) to check one archive by hand: both
minisign signatures, the trusted comment naming that file and `version:$V`, the SHA-256, and

```bash
gh attestation verify "jev-$V-x86_64-unknown-linux-musl.tar.gz" --repo shaharia-lab/jev-cli
```

For a stable release also check the downstreams and a real install:

```bash
# expect jev.rb and jev@<version>.rb
gh api repos/shaharia-lab/homebrew-tap/contents/Formula --jq '.[].name'
curl -fsS "https://crates.io/api/v1/crates/jev-cli" | jq -r '.crate.max_version'
curl -fsS "https://crates.io/api/v1/crates/jev-client" | jq -r '.crate.max_version'

dir=$(mktemp -d)
curl -fsSL https://raw.githubusercontent.com/shaharia-lab/jev-cli/main/install.sh \
  | sh -s -- --install-dir "$dir" --no-modify-path --version "$V"
"$dir/jev" version -o json
"$dir/jev" update --check    # exit 20 means a newer release exists, 0 means up to date
```

## 7. Afterwards

- Say what shipped: the version, the release URL, and whether Homebrew and crates.io were updated
  (a pre-release skips both by design).
- If anything in step 6 failed, open an issue rather than patching the release in place.
- An automatic update from the previous release can only be tested once two real releases exist.

## Pre-releases

Same pipeline, published as a GitHub pre-release that never becomes "latest". Homebrew and
crates.io hang off the `stable` job, which a pre-release skips. Either merge with
`Release-As: 0.2.0-rc.1`, or push the tag `v0.2.0-rc.1` on a commit whose `Cargo.toml` already
says that version. A tag that disagrees with `Cargo.toml` fails the run before anything is built.

Dispatching `release.yml` on a branch is a dry run: everything is built, checked and packed, and
nothing is published. Use it to test a change to the release plumbing.
