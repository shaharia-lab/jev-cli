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
