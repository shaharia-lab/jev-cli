# Fuzz targets

Every parser of input that `jev` does not control, run by [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz)
(libFuzzer). No input may make `jev` panic (PRD REL-7): a panic is a crash here, and a crash is a
bug.

| Target | What it feeds | Entry point |
| --- | --- | --- |
| `request` | a JSON request: `Document` parsing, `validate::check_document` under every option, conversion to a typed `Request` | `jev_client::fuzz::request` |
| `response` | a response body: `Response`, `ModelList`, and the three shapes of an error body | `jev_client::fuzz::response` |
| `request_file` | a request file as JSON and as YAML, prepared as `jev eval` prepares it | `jev_cli::fuzz::request_file` |
| `output` | a response rendered in every output format, with a `--field` path and an `--assert` condition or `--abstain-band` | `jev_cli::fuzz::output` |
| `batch_rows` | a JSONL or CSV batch input, checked as `jev batch run` checks it | `jev_cli::fuzz::batch_rows` |
| `config` | a `config.toml`, which must also read again once jev has written it back, and a `credentials` file | `jev_cli::fuzz::config` |
| `mcp` | lines from an MCP client, answered by a server with no API key | `jev_cli::fuzz::mcp` |

The entry points live in the crates, in `src/fuzz.rs`, so that they can reach private code. They
are compiled only with `--cfg fuzzing` (which cargo-fuzz sets) and in the crates' own tests, which
run each one on a few inputs so that the targets keep compiling. This directory is a Cargo
workspace of its own, so `cargo build`, `cargo test` and `make check` never build it.

## Running

cargo-fuzz needs a nightly toolchain:

```sh
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
fuzz/run.sh request 60          # one target for 60 seconds, in one process
cargo +nightly fuzz list        # the targets
```

`run.sh` starts from the committed seeds in `seeds/<target>/` plus whatever corpus earlier runs
left in `corpus/<target>/` (not committed), and uses the dictionary in `dict/jev.dict`. The
[Fuzz workflow](../.github/workflows/fuzz.yml) runs every target for ten minutes each week and on
demand, and for one minute on a pull request that changes this directory.

## When a target crashes

libFuzzer writes the input to `artifacts/<target>/`, and CI uploads it as an artifact.

1. Reproduce it: `cargo +nightly fuzz run <target> artifacts/<target>/crash-...`.
2. Fix the cause where it is, and add a regression test next to that code that feeds it the same
   input. Tests do not read files from this directory, so that each crate stays self-contained.
3. If the input is a good example of something new, copy it into `seeds/<target>/`.

A fix that is too large for the change at hand gets an issue of its own, with the input attached.
