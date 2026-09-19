# Security policy

`jev` handles an API key and often processes sensitive text, so security reports are taken
seriously and handled with priority.

## Reporting a vulnerability

**Please do not open a public issue for a security problem.** Email:

**hello@shaharialab.com**

Include what you can of the following:

- A description of the problem and its impact.
- Steps to reproduce, or a proof of concept.
- The `jev` version, how it was installed, and your operating system.
- A suggested fix, if you have one.

You will get an acknowledgement within 48 hours. We will work with you to confirm the problem,
agree on a disclosure date, and credit you in the release notes unless you prefer otherwise.

Never include a real API key in a report. If a key was exposed while you were investigating,
revoke it in the [TypeSafe console](https://console.typesafe.ai/keys).

A bug with no security impact belongs in a normal
[GitHub issue](https://github.com/shaharia-lab/jev-cli/issues).

## Supported versions

`jev` is in early development and has no release yet. Once releases exist, security fixes go into
the latest release only, and the built-in updater brings self-managed installs to it.

## Security design

These are the rules the project is built to. Each is enforced by tests or continuous integration
as the corresponding feature lands.

- **The API key never travels through command-line arguments.** It comes from the
  `TYPESAFE_API_KEY` environment variable, or from the `credentials` file that `jev auth login`
  writes (readable only by you; `jev` refuses to use it once anyone else can read it). It is never
  written to `config.toml`, logs, error messages or dry-run output.
- **Request and response bodies are never logged** unless you explicitly ask for it, because the
  `state` you send often contains customer data.
- **HTTPS only**, through `rustls`. Certificate verification cannot be turned off. Plain HTTP is
  accepted for loopback addresses only, for testing.
- **No telemetry.** `jev` contacts the TypeSafe API endpoint you configure and, for updates,
  GitHub Releases for this repository. Nothing else.
- **Updates are verified before they are installed**: a signature and a SHA-256 checksum are
  checked against a public key compiled into the binary, and automatic updates never downgrade.
  The full design, including key rotation, will be documented here when release signing lands.
- **The MCP server cannot touch files by default.** File access must be granted per directory when
  the server is started.
- **No `unsafe` code**: both crates use `#![forbid(unsafe_code)]`.

## Supply chain

- `Cargo.lock` is committed and every build uses `--locked`.
- `cargo deny` gates advisories, licences, banned crates and crate sources on every pull request;
  `cargo audit` runs there too, and both run daily against new advisories.
- Every third-party GitHub Action is pinned to a full commit SHA, checked in CI.
- Commits to `main` must be signed and arrive through a pull request.

`jev` is an unofficial project and is not affiliated with TypeSafe AI. Problems with the TypeSafe
API or platform itself should be reported to TypeSafe AI.
