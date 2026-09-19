# jev

An unofficial command-line tool for [TypeSafe AI](https://typesafe.ai)'s **Jev** model, written in
Rust and built for people, shell scripts and AI agents alike.

> [!IMPORTANT]
> **Unofficial project.** `jev` is community-built. It is not affiliated with, endorsed by, or
> sponsored by TypeSafe AI. "TypeSafe" and "Jev" belong to their owner.

> [!WARNING]
> **Early development.** There is no release yet and the commands below are not implemented.
> Progress is tracked in the [v1 epic](https://github.com/shaharia-lab/jev-cli/issues/2).

## What it will do

Jev never generates text. You send it some content (the *state*) and typed questions, and it
returns calibrated probabilities over answers you defined. That makes it a semantic `if`
statement, which fits the shell well:

```bash
# Exit status 0 when the answer is yes with probability >= 0.7, 10 when it is not
if git log -1 --pretty=%B | jev noul "Does this commit describe a user-facing change?" --fail-under 0.7; then
  echo "changelog entry needed"
fi
```

Planned for the first version:

- `jev eval`, `jev noul`, `jev choice`, `jev score` with answers mapped to exit codes
- Offline validation before anything is sent or billed
- `jev batch run` over JSONL and CSV with bounded concurrency, retries and resume
- Human output on a terminal, JSON when piped, and stable exit codes
- For AI agents: intent-oriented help, a machine-readable command spec, JSON Schemas, and an MCP
  server mode
- API key in the operating system keychain, no telemetry, signed and verified self-updates

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) to build and test, and [`CLAUDE.md`](CLAUDE.md) for the
project guide. Report security problems privately as described in [`SECURITY.md`](SECURITY.md).

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
this project by you, as defined in the Apache-2.0 licence, shall be dual licensed as above, without
any additional terms or conditions.
