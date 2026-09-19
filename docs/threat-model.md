# Threat model

What `jev` protects, from whom, and where each protection is enforced and tested. It is the
long form of the rules summarised in [SECURITY.md](../SECURITY.md), written for reviewers and
for anyone changing the code. Report a vulnerability as SECURITY.md describes, not in a public
issue.

Last reviewed: 2026-09, against every shipped command, the batch engine, the MCP server, the
updater, the install scripts and the release workflows.

## What is worth protecting

| Asset | Why it matters |
| --- | --- |
| The TypeSafe API key | It spends money and reads an account's data. |
| `state` and answers | `state` is usually customer data: tickets, code, messages. |
| The `jev` binary and its updates | It runs on a developer's machine and in CI, and it replaces itself. |
| The user's files | `jev` reads what it is pointed at and writes only where it is told, including as an MCP server driven by an agent. |

## Who `jev` does not trust

1. **The API endpoint.** It is reached over TLS, but what it sends back is data, never
   instructions: responses are parsed tolerantly, and a body that echoes the key or `state` must
   not turn that into a leak.
2. **Files it is given.** Request files, question sets, batch inputs and their rows come from
   repositories, exports and customers, and may be hostile.
3. **MCP clients.** The agent on the other end of `jev mcp serve` chooses tool arguments,
   including paths.
4. **Anything downloaded.** A release archive, its checksums and its signatures are untrusted
   until verified against a key compiled into the binary.
5. **Pull requests.** CI runs code from a fork; nothing it can influence may reach a secret.

`jev` does trust the machine it runs on and the person running it. A compromised account, a
hostile `PATH`, or a debugger attached to the process are outside what a CLI can defend against.

## Trust boundaries

### 1. The environment and the configuration directory

The key comes from `TYPESAFE_API_KEY`, which always wins, or from the `credentials` file that
`jev auth login` writes: `0600` from its first byte, in a `0700` directory, refused with the exact
`chmod` when anyone else can read it. There is no OS keychain and no browser flow (PRD D9). In
memory the key is a zeroize-on-drop type whose `Debug` prints `[REDACTED]` and which has no
`Display`; the only way to read it is one reviewable method. It is never accepted as a
command-line argument value, so it cannot reach shell history or `ps`.

`config.toml` never holds a key, and `jev config set` refuses one. Settings come from one
resolver, so no command reads a secret from somewhere of its own. `jev` has no project-local
configuration: a repository cannot redirect the key to another host by checking in a file.

*Accepted:* on Windows the permission check is not made, because the ACL model has no simple
equivalent of mode `0600`; the file lives in the per-user profile directory. Also accepted: a
person who passes `--base-url` or sets `TYPESAFE_BASE_URL` sends their key where they said, and
`--insecure-allow-http` warns on every call that it travels unencrypted.

### 2. The API over the network

`https://` only, except loopback; TLS through `rustls` with the platform verifier, built
explicitly, and certificate verification cannot be switched off. Redirects are never followed, so
the `Authorization` header cannot be replayed to another host. The header is marked sensitive, so
the HTTP stack does not print it even at `TRACE`.

Nothing from a response is trusted:

- The whole response body is scrubbed of the key before anything reads it, so a server that
  echoes the header back cannot get it into `--raw` output, an unknown answer kept as raw JSON,
  an error message or a log line.
- Bodies are logged only with `--debug-bodies`, which warns that `state` may be in them. No
  verbosity level logs a body by itself.
- Error text is never quoted from a body of unknown shape; a 422 echoes the request, and only its
  summary is shown.
- Control characters in anything `jev` prints for a person are written out as escapes, so no text
  `jev` did not write itself can drive the terminal (CWE-150). That covers errors and notices, the
  answer envelope, tables, the model list, validation findings and a batch plan, whether colour is
  on or off. The machine formats keep the value exactly as it was sent, since JSON and YAML escape
  control characters themselves and a program needs the value unchanged; so do `--field`,
  `jev config get` and `jev config path`, each of which prints one raw value for a script.

### 3. Files the user names

Request files are parsed with a YAML reader that caps how far aliases may expand, so an alias
bomb is refused rather than exhausting memory. Every request is validated offline before anything
is sent, and `--dry-run` neither touches the network nor needs a key. `state` and answers are
written only to paths the user names (`--out`, `--summary-json`, a redirected stdout); nothing
else on disk keeps them. The only other file `jev` writes by itself is the automatic-update state
in the configuration directory, which holds a timestamp and what the last check found.

### 4. MCP clients

`jev mcp serve` speaks JSON-RPC on stdin and stdout to whoever started it; it opens no port and
has no other caller. File access is deny-by-default: without `--allow-dir` the `batch_run` tool is
neither listed nor callable, and with it every path an agent gives is resolved and confined to the
allowed roots, symbolic links included, before anything is read, written or sent. A client cannot
choose the base URL, the profile or the key, and cost limits are the operator's, not the client's.
Tool results carry answers and errors, never the key.

### 5. GitHub Releases, the updater and the install scripts

`jev update`, the background check and the install scripts download only from this repository's
releases, over HTTPS, following redirects only to GitHub's own asset hosts. Before anything is
written, the signature of `SHA256SUMS` and of the archive are checked against the two public keys
compiled into the binary, each signature's trusted comment must name that file and that version,
and the archive's SHA-256 must match. Only the binary is read out of the archive, into memory: no
path from an archive is ever used on disk. A new binary must pass a self-test before it is kept,
and it runs that self-test without the API key in its environment. Automatic updates never
downgrade, are off for package-manager installs and in CI, and can be turned off in one setting.

The background check is a detached child with no standard streams and with `TYPESAFE_API_KEY`
removed from its environment: it talks to GitHub, and it has no reason to hold a key.

The install scripts verify checksums always and signatures when `minisign` is installed
(`--require-signature` makes a missing `minisign` an error), run the new binary once before
installing it, and never ask for elevation.

*Accepted:* if both signing keys were compromised at once, installed copies cannot be moved to
new keys by an update; SECURITY.md says what happens then. Without `minisign`, an install script
proves only that the download matches the release's own checksum file.

### 6. No telemetry

The only hosts `jev` contacts are the configured TypeSafe base URL and GitHub Releases for this
repository. Nothing is collected, and there is no crash reporting.

### 7. Continuous integration and the release workflow

Every workflow starts from no permissions or `contents: read` and raises them per job; none uses
`pull_request_target`; every third-party action is pinned to a full commit SHA, checked in CI; and
checkouts do not persist credentials. Values from an event, such as a pull-request title, are
passed to scripts through the environment, never interpolated into a shell line.

Releases are built from a clean slate with no build cache. Signing happens in one job that runs
only for a `v*` tag, in a protected deployment environment a maintainer approves, and the signing
secrets are available to that job alone. The credentials that publish the Homebrew tap and the
release pull request come from deployment environments of their own, so a workflow run on any
other branch cannot read them. A separate job re-downloads every published asset and
verifies signatures, checksums and build provenance before the release leaves draft, and the later
stages depend on it.

### 8. Dependencies

`Cargo.lock` is committed and every build uses `--locked`. `cargo deny` and `cargo audit` gate
advisories, licences, sources and banned crates on every pull request and daily. `jev-client` may
not depend on CLI, terminal or configuration crates, which CI checks. Both crates
`#![forbid(unsafe_code)]`. There is no OpenSSL or `native-tls`, and no crate source but crates.io.

## How this is verified

| Guarantee | Where it is tested |
| --- | --- |
| No command shows or writes the key, from either source, at maximum verbosity, in success and failure | `crates/jev-cli/tests/secret_leak.rs` |
| `state` reaches stderr only with `--debug-bodies`, and disk only where it was asked to | `crates/jev-cli/tests/secret_leak.rs` |
| Only the API transport and the release client can open a connection, and only to known hosts | `crates/jev-cli/tests/secret_leak.rs` |
| Nothing reaches a host that is neither the API nor GitHub (every proxy variable is a trap) | `crates/jev-cli/tests/secret_leak.rs` |
| The background update check runs without the key in its environment | `crates/jev-cli/tests/secret_leak.rs` |
| The key survives no log, error or `Debug` at `TRACE`, and an echoed key is scrubbed | `crates/jev-client/tests/http_transport.rs` |
| The credentials file is `0600`, and a wider one is refused with the `chmod` to run | `crates/jev-cli/tests/auth.rs` |
| MCP file access is confined to `--allow-dir`, symbolic links included | `crates/jev-cli/tests/mcp.rs` |
| A release is installed only when its signature, trusted comment and checksum all match | `crates/jev-cli/tests/update.rs`, `crates/jev-cli/tests/install_script.rs` |
| Actions are pinned, and the public keys in the scripts match the committed ones | `scripts/ci/check-action-pins.sh`, CI |

The leak test derives its command list from `jev spec`, so a new command is covered the day it is
added: it always gets `--help`, a bare run and an unknown flag, and the test fails until the
command also has a scenario of its own.

## Findings of this review

| Finding | Severity | Status |
| --- | --- | --- |
| A response body that echoed the `Authorization` header reached `--raw` output and any unknown answer kept as raw JSON | Low (needs a hostile or misbehaving endpoint) | Fixed: the body is scrubbed before it is parsed, logged or kept |
| Terminal escape sequences in a server's error message, or in text quoted from a file, were printed to the terminal unchanged | Low | Fixed: errors and notices here, and the rest of human output in [#80](https://github.com/shaharia-lab/jev-cli/issues/80) |
| The self-test of a newly downloaded binary ran with the API key in its environment | Informational | Fixed: the probe removes it, as the background check already did |
| The GitHub App credentials that publish the Homebrew tap and the release pull request were repository-level secrets, so any workflow on a branch could read them | Low (requires write access to this repository) | Fixed in [#83](https://github.com/shaharia-lab/jev-cli/pull/83): both jobs now take them from a deployment environment |

Nothing else in this review changed behaviour: the key handling, the updater's trust chain, the
MCP confinement and the workflow permissions held up as documented.
