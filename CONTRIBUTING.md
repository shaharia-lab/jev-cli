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
