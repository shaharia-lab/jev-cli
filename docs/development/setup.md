# Development setup

Everything you need to build `jev`, run what CI runs, and get a change ready for review. The
contribution policy itself is in [CONTRIBUTING.md](../../CONTRIBUTING.md), and
[CLAUDE.md](../../CLAUDE.md) is the architecture guide that contributors and AI agents both work
from.

## Get the toolchain

You need [rustup](https://rustup.rs). The toolchain is pinned in `rust-toolchain.toml`, and
rustup installs it the first time you run `cargo` here.

```bash
git clone https://github.com/shaharia-lab/jev-cli
cd jev-cli
cargo build --workspace
cargo run -p jev-cli -- --help
```

Two extra tools are needed for the dependency gates, once per machine:

```bash
cargo install --locked cargo-deny cargo-audit
```

## Install the git hooks

Once per clone. They need [pre-commit](https://pre-commit.com/#install):

```bash
pre-commit install     # or: make hooks
```

On commit: file hygiene, `cargo fmt --check`, `cargo clippy`, and a guard that refuses commits to
`main`. On push: the test suite and the crate-boundary check. Never bypass them with
`--no-verify`. Fix what they report; a worktree shares the main checkout's hooks, so they run
there too.

## The everyday loop

```bash
cargo test --workspace --all-features --locked     # the whole suite
cargo test -p jev-cli --all-features --test eval   # one test file
cargo run -p jev-cli -- noul "Is this angry?" --state "..." --dry-run
make check                                         # everything CI runs
```

`make check` is the gate to run before every push: formatting, clippy for all-features and for
default-features, tests for both, the minimum supported Rust version, documentation warnings,
`cargo deny`, `cargo audit`, the repository policy scripts and a packaging dry run.

## The make targets

| Target | What it does |
| --- | --- |
| `make check` | Every gate CI runs |
| `make msrv` | Check against the minimum supported Rust version |
| `make reference` | Regenerate `docs/commands.md` from the command tree |
| `make schemas` | Regenerate the published JSON Schemas in `schemas/` |
| `make dist-assets` | Write man pages and shell completions to `target/dist-assets/` |
| `make bench` | The performance budgets, on a release build |
| `make package` | `cargo publish --workspace --dry-run` |
| `make hooks` | Install the pre-commit hooks |

Two checks are deliberately outside `make check`, because they need a release build or a nightly
toolchain: the performance budgets (`make bench`) and fuzzing (`fuzz/run.sh <target> [seconds]`,
see [fuzz/README.md](../../fuzz/README.md)).

## Things that regenerate

Some files are generated, and a test fails when the committed copy is stale. Never edit them by
hand:

| File | Regenerate with |
| --- | --- |
| `docs/commands.md` | `make reference` |
| `schemas/*.json` | `make schemas` |
| Help and spec snapshots in `crates/jev-cli/tests/snapshots/` | `JEV_UPDATE_SNAPSHOTS=1 cargo test -p jev-cli --all-features --test help` |

Review the diff after regenerating. A change you did not intend is a bug in the change you did.

## Testing without an API key

You never need a real key. Tests run against a local [wiremock](https://docs.rs/wiremock) server
and default to an address nothing listens on, so a forgotten mock cannot reach the real API.
Offline commands (`jev validate`, `jev schema`, `jev spec`, `--dry-run`) need no key either.

The opt-in live smoke suite (`crates/jev-cli/tests/live.rs`) is ignored by default and runs
nightly in CI. See [Testing](testing.md).

## Where things are

```text
crates/jev-client/   the library: typed requests, validation, HTTP transport
crates/jev-cli/      the jev binary: commands, config, output, batch, MCP, updater
docs/                user guide, developer guide, generated command reference
schemas/             published JSON Schemas (generated)
scripts/             CI helpers and release scripts
skills/jev-cli/      the agent skill
fuzz/                cargo-fuzz targets, a workspace of its own
```

[Architecture](architecture.md) explains how they fit together.

## Before you open a pull request

- One issue per pull request, Conventional Commit title, signed commits.
- `make check` passes.
- Help text, `docs/`, the README and the agent skill updated when your change makes them stale.
- The full checklist is in [CONTRIBUTING.md](../../CONTRIBUTING.md).
