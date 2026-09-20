# jev-client

Unofficial Rust client for [TypeSafe AI](https://typesafe.ai)'s **Jev** model: typed requests and
answers, offline validation, retries and typed errors.

Jev never generates text. A request carries a `state` and a map of typed questions (`noul` for a
yes/no probability, `choice` for one of up to 255 options, `score` for a position on a rubric),
and the answers come back as calibrated probabilities.

This is the library behind the [`jev` command-line tool](https://github.com/shaharia-lab/jev-cli).
It has no command-line, terminal or configuration-file concerns, so it can be used on its own.
Validation runs offline and reports every problem at once, with a rule id, a JSON Pointer path
and a suggested fix, so a request can be checked before it is ever sent.

The API reference is on [docs.rs](https://docs.rs/jev-client). What the server actually does,
including where it differs from its own documentation, is recorded in
[api-behaviour.md](https://github.com/shaharia-lab/jev-cli/blob/main/docs/development/api-behaviour.md).

**Status:** before 1.0. The public API is not stable yet; public structs are `#[non_exhaustive]`
so that adding a field is not a breaking change.

This project is not affiliated with, endorsed by, or sponsored by TypeSafe AI.

## Licence

Licensed under either of [Apache License, Version 2.0](https://github.com/shaharia-lab/jev-cli/blob/main/LICENSE-APACHE)
or [MIT license](https://github.com/shaharia-lab/jev-cli/blob/main/LICENSE-MIT) at your option.
