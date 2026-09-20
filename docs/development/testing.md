# Testing

`jev` is tested against behaviour a user can see: stdout, stderr **and** the exit code. Network
calls go to a local mock, so no test ever needs a real API key.

```bash
cargo test --workspace --all-features --locked     # everything
cargo test -p jev-cli --all-features --test batch  # one file
cargo test -p jev-client --locked                  # the library only
make check                                         # what CI runs
```

CI runs the suite twice: once with `--all-features`, which turns on the test hooks, and once
with default features, which proves a release build does not contain them.

## How a CLI test is written

`assert_cmd` runs the real binary, [wiremock](https://docs.rs/wiremock) stands in for the API,
and every test isolates its own state:

- Point `JEV_CONFIG_DIR` at a scratch directory. A test must never touch a person's real
  configuration or credentials.
- Clear the environment a developer's shell might carry (`CI`, `NO_COLOR`, `TERM`, `LANG`,
  `JEV_*`, `TYPESAFE_*`).
- Assert stdout, stderr and the exit code, not just one of them.
- Test the negative path too, and check that a test would fail if the fix were reverted.

Nothing sleeps: the transport takes a fake `Clock`, and the updater a fake `Waiter`, so retries
and back-off are tested in milliseconds.

## What each test file guards

| File | Guards |
| --- | --- |
| `crates/jev-cli/tests/docs.rs` | The README and the pages in `docs/`: every command, flag and setting they name exists, their exit-code tables are the spec's, their request files validate, and the README quick start runs against a mock |
| `crates/jev-cli/tests/skill.rs` | The agent skill, the same way, plus the MCP tool list |
| `crates/jev-cli/tests/help.rs` | Every `--help` and `jev spec`, as snapshots, and the help lint |
| `crates/jev-cli/tests/secret_leak.rs` | The key reaching no stream and no file, for every command `jev spec` lists, at maximum verbosity, with and without `--debug-bodies` |
| `crates/jev-cli/tests/escape.rs` | A sentinel escape sequence never reaching a terminal unescaped |
| `crates/jev-cli/tests/terminal.rs` | What a person sees, on a real pseudo-terminal (Unix only) |
| `crates/jev-cli/tests/batch.rs` | The batch engine: ordering, resume, interruption, back-off, memory |
| `crates/jev-cli/tests/mcp.rs` | The MCP protocol loop, the tools, and the `--allow-dir` boundary |
| `crates/jev-cli/tests/update.rs` | Staged updates against signed fake releases, rollback, and the guards |
| `crates/jev-cli/tests/install_script.rs` | `install.sh` and `install.ps1` against a local server, including tampered assets |
| `crates/jev-cli/tests/performance.rs` | The start-up and request-overhead budgets (ignored by default) |
| `crates/jev-client/tests/http_transport.rs` | Retries, error mapping, and the sentinel-key leak test at `TRACE` |
| `crates/jev-client/tests/validation.rs` | Every validation rule, which cannot exist without a case here |

## Snapshots

Help output and the command spec are snapshot tests. After an intended change:

```bash
JEV_UPDATE_SNAPSHOTS=1 cargo test -p jev-cli --all-features --test help
```

Then read the diff. `docs/commands.md` and `schemas/` are generated in the same spirit: a test
fails when the committed copy is not what this build renders, and `make reference` and
`make schemas` refresh them.

## Fuzzing

Every parser of outside input has a fuzz target. `fuzz/` is a workspace of its own and needs a
nightly toolchain:

```bash
fuzz/run.sh request 60
```

Entry points live in each crate's `src/fuzz.rs`, behind `#[cfg(any(test, fuzzing))]`, so they
reach private code and the ordinary test runs keep them compiling. A crash is fixed with a
regression test next to the code, not only in the corpus. CI fuzzes every target weekly.
See [fuzz/README.md](../../fuzz/README.md).

## Performance budgets

`make bench` runs them on a release build: start-up time, request overhead, and batch memory over
a million rows. They are `#[ignore]`d so they never slow the ordinary suite, and CI runs them on
Linux for every pull request. The guard values come from CI's runner, so a budget that trips is
a finding to investigate, not a number to raise.

## Live API tests

`crates/jev-cli/tests/live.rs` is the only suite that talks to the real API. It is `#[ignore]`d,
needs `TYPESAFE_API_KEY`, and runs nightly in CI, where it files a tracking issue when it fails.
Its job is to notice the server changing: it asserts the quirks recorded in
[How the API actually behaves](api-behaviour.md), so drift shows up as a failing assertion rather
than as a confused user.

Never assert an exact probability. Answers are not bit-for-bit deterministic, so tests assert
shapes and ranges.

## Adding a test with a feature

- A new command needs a scenario in `tests/secret_leak.rs`; the test names it until it has one.
- A new validation rule needs a case in `tests/validation.rs`.
- A new surface that could carry the key needs its own sentinel test.
- A change to the quick start in the README needs the same change to `QUICK_START` in
  `tests/docs.rs`.
