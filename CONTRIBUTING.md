# Contributing to jev

Thanks for helping. `jev` is an unofficial, community-built CLI for TypeSafe AI's Jev model, and
it aims to be as dependable as the tools it is modelled on (`gh`, `aws`, `gcloud`).

Read [`CLAUDE.md`](CLAUDE.md) first. It is the project guide for people and AI agents alike:
product rules, security rules, architecture and the facts about the API that code must respect.

## Getting set up

You need [rustup](https://rustup.rs). The toolchain is pinned in `rust-toolchain.toml` and rustup
installs it the first time you run `cargo` in this repository.

```bash
git clone https://github.com/shaharia-lab/jev-cli
cd jev-cli
cargo build --workspace
cargo run -p jev-cli        # runs the `jev` binary
```

Then install the git hooks, once per clone. They need [pre-commit](https://pre-commit.com/#install):

```bash
pre-commit install          # or: make hooks
```

This installs two hooks. On **commit**: file hygiene (whitespace, line endings, YAML/TOML syntax,
large files, private keys), `cargo fmt --check` and `cargo clippy` when Rust or Cargo files change,
and a guard that stops commits to `main`. On **push**: the test suite and the crate-boundary check.
If a hook fixes a file for you, stage the fix and commit again.

For the dependency checks, install two extra tools once:

```bash
cargo install --locked cargo-deny cargo-audit
```

## Before you open a pull request

Run everything CI runs:

```bash
make check
```

Or run the gates one at a time:

| Gate | Command |
| --- | --- |
| Formatting | `cargo fmt --all --check` (fix with `cargo fmt --all`) |
| Lints | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` |
| Tests | `cargo test --workspace --all-features --locked` |
| Minimum Rust version | `make msrv` |
| Documentation | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features --locked` |
| Advisories, licences, bans, sources | `cargo deny --locked check` |
| Vulnerabilities | `cargo audit --deny unsound --deny yanked` |
| Repository policy | `scripts/ci/check-action-pins.sh` and `scripts/ci/check-client-deps.sh` |

CI runs lints and tests on Linux, macOS and Windows.

Two more checks are not part of `make check`, because they need a release build or a nightly
toolchain:

- **Performance budgets** (`make bench`): start-up and request overhead, and batch memory over a
  million rows, on a release build. CI runs them on Linux for every pull request.
- **Fuzzing** (`fuzz/run.sh <target> [seconds]`): see [fuzz/README.md](fuzz/README.md). CI fuzzes
  every target for ten minutes each week.

## How work is organised

- Work is tracked as GitHub issues under an epic. Issues are ordered with GitHub's native
  **Blocked by** relationships: pick one whose blockers are all closed.
- One issue, one pull request. Keep it in scope; note unrelated findings in a new issue.
- `main` requires a pull request, **signed commits** and linear history. Set up
  [commit signing](https://docs.github.com/en/authentication/managing-commit-signature-verification)
  before your first commit.
- Pull request titles follow [Conventional Commits](https://www.conventionalcommits.org):
  `feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `chore:`. Pull requests are squash-merged, so
  the title becomes the commit on `main` and feeds the changelog.
- Reference the issue (`Closes #123`) in the description.

## Releases

Releases are cut by [release-please](https://github.com/googleapis/release-please); nobody edits
versions or `CHANGELOG.md` by hand.

1. Every merge to `main` updates a release pull request with the next version, worked out from the
   Conventional Commit titles since the last release, and its changelog entry. A `feat` bumps the
   minor version, a `fix` the patch; before 1.0 a breaking change (`!`) bumps the minor version.
   The `PR title` check rejects a pull request whose title does not follow the format.
2. Merging the release pull request tags `vX.Y.Z` and creates a **draft** GitHub Release. The
   tag starts `.github/workflows/release.yml`. release-please works through the
   `jev-release-bot` GitHub App (`RELEASE_APP_ID`, `RELEASE_APP_PRIVATE_KEY`, held in the
   `release-please` environment, which admits `main` only), so its pull request gets CI and its
   tag triggers the release.
3. That workflow builds all six targets (x86_64 and aarch64 of static-musl Linux, macOS and
   Windows) on native runners, checks each binary's size, version and (on Linux) static linking,
   packs `jev-<version>-<triple>.tar.gz` (`.zip` on Windows) with the licences, man pages and
   shell completions, and adds the JSON Schemas, a CycloneDX SBOM and `SHA256SUMS`. The owner
   approves the `release` environment, the only place the signing key is available; that job
   signs every asset and `SHA256SUMS` with minisign and uploads them to the draft. Build
   provenance is attested for every asset and binary. The assets are then downloaded again and
   verified (checksums, signatures against `crates/jev-cli/keys/release-primary.pub` with the
   standard minisign CLI, and attestations), and only then is the release published.
4. A stable release then goes to the Homebrew tap, using the tap App's credentials from the
   `publish` environment, which admits `v*` tags only, and to crates.io: `jev-client` first, then
   `jev-cli` once the index has `jev-client`, through `scripts/release/crates-publish.sh` with the
   `CARGO_REGISTRY_TOKEN` of the `release` environment (so the owner approves that job as well).
   A crates.io version can never be replaced, so the script skips a crate whose version is
   already there, and re-running the job after a partial publish is safe.
   `[package.metadata.binstall]` in `crates/jev-cli/Cargo.toml` points `cargo binstall` at the
   release archives and the primary signing key. Every pull request runs
   `cargo publish --workspace --dry-run` (`make package`), which builds both crates from exactly
   what would be uploaded; `crates/jev-cli` ships only `src/`, `build.rs`, the public keys, its
   README and the licences.

The install scripts at the repository root, `install.sh` and `install.ps1`, download from GitHub
Releases and are served to users straight from `main`, so a change to them ships when it is
merged. `crates/jev-cli/tests/install_script.rs` runs them on every OS against signed fake
releases on a local server (it needs `minisign`; CI installs it with
`scripts/ci/install-minisign.sh`), and `scripts/ci/lint-install-scripts.sh` runs ShellCheck and
PSScriptAnalyzer on them.

`scripts/release/self-test.sh` (part of `make check` and CI) checks the committed public keys and
drives signing and verification with a throwaway key, including tampered assets. Key rotation and
manual verification are described in [`SECURITY.md`](SECURITY.md).

A failed run is resumed with **Re-run failed jobs**, or with
`gh workflow run release.yml --ref vX.Y.Z` once it is too old to re-run. Publishing can run any number of times: a
draft gets exactly the new assets, and a published release is never changed.

A **pre-release** (`0.2.0-rc.1`) exercises the same pipeline and is published as a GitHub
pre-release that never becomes "latest". Stages that must not see pre-releases (crates.io, the
Homebrew formula) depend on the `stable` job, which is skipped for them. Either merge a commit
with a `Release-As: 0.2.0-rc.1` footer and let release-please do the rest, or push the tag
`v0.2.0-rc.1` on a commit whose `Cargo.toml` already says that version.
A tag that disagrees with `Cargo.toml` fails the run before anything is built.

Dispatching `release.yml` on a branch, or a pull request that touches the release plumbing, is a
dry run: everything is built, checked and packed, and nothing is published.

## Code expectations

- **Two crates, one direction.** `crates/jev-client` is a standalone library with no
  command-line, terminal or configuration-file dependencies. `crates/jev-cli` is the `jev` binary
  and depends on it. CI enforces the boundary.
- **Tests assert behaviour users see.** For commands, that means stdout, stderr **and** the exit
  code. Network calls go to a local mock server; tests never need a real API key.
- **No panics on user input.** `unwrap`, `expect`, `panic!` and slice indexing are linted outside
  tests. Return an error that maps to a documented exit code.
- **No `unsafe`.** Both crates forbid it.
- **Secrets never reach output.** No API keys in arguments, logs, errors or fixtures.
- **Help text is part of the feature.** Every command explains when to use it, shows examples and
  lists its exit codes, because AI agents choose commands from that text.
- Match the surrounding code's naming, idiom and comment density. Document public items in
  `jev-client`.

## User documentation

The pages a user reads are [`README.md`](README.md) and the three in [`docs/`](docs/README.md):
the command reference, the exit-code and JSON contract, and configuration. They must stand on
their own, so never link them to a page written for contributors, and never to a planning document:
the PRD and the study notes that shaped v1 were removed for `0.1.0`, and a test keeps them from
coming back. If a decision's rationale is needed, it is in this repository's history.

Two pages in `docs/` are written for contributors rather than users:
[`api-behaviour.md`](docs/api-behaviour.md), which records what the TypeSafe API actually does
where that differs from its documentation, and [`threat-model.md`](docs/threat-model.md). Update
`api-behaviour.md` whenever the nightly live smoke test catches the server changing.

[`docs/commands.md`](docs/commands.md) is **generated** from the command tree by
`crates/jev-cli/src/reference.rs`, like the man pages. Never edit it by hand; after changing a
command, a flag or its help, run:

```bash
make reference
```

`crates/jev-cli/tests/docs.rs` holds all of it to the binary: the committed reference must be
exactly what this build renders, every `jev` command and flag the pages name must exist in
`jev spec`, their exit-code tables must be the spec's, every setting they name must be real (and
every real setting must be documented), every request file they show must pass
`jev validate --strict`, and the README's quick start is run, as written, against a mock of the
API. When you
change the quick start, change `QUICK_START` in that test with it.

## The agent skill

[`skills/jev-cli/SKILL.md`](skills/jev-cli/SKILL.md) teaches an AI agent to use `jev`. It
installs through `npx skills add` and as a Claude Code plugin from
[`.claude-plugin/marketplace.json`](.claude-plugin/marketplace.json). It teaches a workflow and
points at `jev spec`, `--help` and `jev schema` for the contract, rather than restating it.

`crates/jev-cli/tests/skill.rs` holds it to the binary: every `jev` command and flag it names must
exist and be implemented in `jev spec`, its exit-code table must be the spec's, the MCP tools it
lists must be the ones `jev mcp serve` offers, and every request file it shows must pass
`jev validate --strict`. When a change breaks one of these, update the skill in the same pull
request. Do not name a flag or command before it has merged.

To check both ways of installing it from a checkout, without touching your own setup:

```bash
CLAUDE_CONFIG_DIR="$(mktemp -d)" sh -c 'claude plugin marketplace add "$PWD" &&
  claude plugin install jev@jev-cli && claude plugin details jev@jev-cli'
(cd "$(mktemp -d)" && npx skills add "$OLDPWD" --skill jev-cli -a claude-code -y && ls .claude/skills)
```

### Agent smoke test

The v1 success metric is that an agent given only `jev spec` and `jev schema request` writes a
valid request and runs it successfully at the first attempt, in at least 9 of 10 trials. Rerun it
when the spec, the schemas or the help change substantially, and record the result in the pull
request.

1. Save `jev spec -o json` and `jev schema request -o json` into an empty directory, with one
   subdirectory per trial holding any input file the task needs.
2. Give each of ten fresh agents one task (below), and these rules: read only those two files and
   the trial's own directory; do not run `jev` or search elsewhere; write the request file into the
   trial directory and the single command, with relative paths, to `command.sh`.
3. Run each `command.sh` once, from its directory, with a real key. A trial passes when the
   command exits 0 (or 10 when the task asks for a gate that turns out false), stdout is what the
   task asked for, and the command follows every instruction in the task.

| # | Task |
| --- | --- |
| 1 | One call: is the ticket in `ticket.txt` urgent, and which team (billing, technical, other) handles it; JSON out |
| 2 | Rate how positive the review in `review.txt` is on five levels, with a score question |
| 3 | A self-contained request file with its own state (a commit message) and a pinned model: is it a breaking change? |
| 4 | A JSON object state in its own file (an email), sent as an object; which register: formal, informal, other |
| 5 | A YAML file with a noul, a choice and a score about a job posting passed on the command line |
| 6 | A CI gate: exit status says whether P(yes) is at least 0.6, using `jev`'s own assertion, no parsing |
| 7 | Three separate nouls (harassment, spam, personal data) about a comment stored in the request file |
| 8 | Rate the risk of the change in `diff.txt` on three described levels, printing only the number |
| 9 | From `notes.md`: is there an action item with an owner, and who owns the first one (or nobody) |
| 10 | Two questions about a recipe supplied on stdin through a pipe |

## Dependencies

Adding a dependency is a decision. Prefer well-maintained crates with permissive licences
(`deny.toml` lists what is allowed), avoid anything that pulls in OpenSSL, and keep `jev-client`
lean. Always commit the updated `Cargo.lock`.

## Minimum supported Rust version

The MSRV is `rust-version` in the root `Cargo.toml`. The policy is latest stable minus two
releases. It is bumped deliberately, in a minor release, never as a side effect.

## Security

Report vulnerabilities privately as described in [`SECURITY.md`](SECURITY.md), not in an issue.

## Licence

Contributions are dual licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at the
user's option, without additional terms.
