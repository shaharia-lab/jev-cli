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

Security fixes go into the latest release only. There are no long-term support branches before
`1.0`, and the built-in updater brings self-managed installs to the fix on their next run.

## Security design

These are the rules the project is built to, each enforced by tests or continuous integration.
[`docs/threat-model.md`](docs/threat-model.md) is the long form: what is worth protecting, which
inputs are not trusted, where each protection lives, and the findings of the last review.

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
  checked against the release public keys compiled into the binary, and automatic updates never
  downgrade. See [Release signing](#release-signing) below.
- **The MCP server cannot touch files by default.** File access must be granted per directory when
  the server is started.
- **Nothing an endpoint sends is trusted.** A response body that echoes the key back is scrubbed
  before anything reads it, and a message shown to a person cannot drive the terminal.
- **No `unsafe` code**: both crates use `#![forbid(unsafe_code)]`.

## Verifying a release

Every file attached to a [GitHub Release](https://github.com/shaharia-lab/jev-cli/releases) is
signed with [minisign](https://jedisct1.github.io/minisign/): an asset `<name>` comes with
`<name>.minisig`. A release holds one archive per platform, the JSON Schemas, a CycloneDX SBOM
(`jev-<version>.cdx.json`), and `SHA256SUMS`, which lists the checksum of every other asset and is
signed as well.

The release public keys:

| Key | Key id | Public key |
| --- | --- | --- |
| Primary (signs every release) | `3CE95B4C3B9987B8` | `RWS4h5k7TFvpPJt8wAoJCpoWgejSO8eVKRs+CqEdFIQAP2E0QwyyDDcU` |
| Next (for rotation, signs nothing yet) | `A3D8DB2C6870B308` | `RWQIs3BoLNvYo0e6bXIBfp7hoYpwkQSJuAWcSD7itrj0ULCXWULmDMx4` |

The same keys are in the repository as
[`crates/jev-cli/keys/release-primary.pub`](crates/jev-cli/keys/release-primary.pub) and
[`release-next.pub`](crates/jev-cli/keys/release-next.pub), and embedded in the install scripts;
CI fails if any of them and this table ever differ.

To verify a download by hand (Linux or macOS; minisign also has Windows builds):

```sh
version=0.1.0 target=x86_64-unknown-linux-musl    # see the release for the file names
base=https://github.com/shaharia-lab/jev-cli/releases/download/v$version
archive=jev-$version-$target.tar.gz
for file in "$archive" SHA256SUMS; do
  curl -fsSLO "$base/$file" && curl -fsSLO "$base/$file.minisig"
done

key=RWS4h5k7TFvpPJt8wAoJCpoWgejSO8eVKRs+CqEdFIQAP2E0QwyyDDcU
minisign -Vm "$archive" -P "$key"
minisign -Vm SHA256SUMS -P "$key"
sha256sum --check --ignore-missing SHA256SUMS    # macOS: shasum -a 256 --check --ignore-missing SHA256SUMS
```

Each `minisign` command must print `Signature and comment signature verified` and a trusted
comment holding `file:<name>` and `version:<version>`: check that they are the file and version
you downloaded, because the comment is what ties a signature to one file of one release. The checksum
command must print `OK` for the archive.

Each release also has [build provenance](https://docs.github.com/actions/security-for-github-actions/using-artifact-attestations)
attestations, which prove that an asset was built from this repository by its release workflow.
With the [GitHub CLI](https://cli.github.com/), for an archive or for the `jev` binary inside it:

```sh
gh attestation verify "$archive" --repo shaharia-lab/jev-cli
```

## Release signing

- Releases are built and signed by the release workflow in this repository, never on a
  maintainer's machine. Signing happens in one job, which runs only for a `v*` tag and only after
  a maintainer has approved that release in a protected deployment environment. The encrypted
  secret key and its password are available to that job alone: not to pull requests, and not to
  the other jobs of the release.
- A signature's trusted comment names the file and the version it was made for, so a valid
  signature cannot be passed off for another file or replayed from an older release.
- The release stays a draft, invisible to users and to the updater, until a separate job has
  downloaded every published asset and checked it with the standard minisign CLI against the
  committed primary key, checked every checksum, and verified the build provenance attestations.
  If any check fails, nothing is published and no later stage (crates.io, Homebrew) runs.
- `jev update` compiles in both public keys, primary and next, and accepts a release signed by
  either of them. Nothing else can change which keys it trusts. Before anything is written it
  checks the signature of `SHA256SUMS` and of the archive, that each signature's trusted comment
  names exactly that file and that version, and the archive's SHA-256. It downloads only from
  GitHub Releases of this repository, over HTTPS, keeps the previous binary for
  `jev update --rollback`, and puts it back by itself if the new binary fails its self-test.
  It never moves to an older version unless asked to with `jev update --version`.
- The install scripts (`install.sh`, `install.ps1`) embed the same two public keys. They always
  check the archive's SHA-256 against `SHA256SUMS`, and, when the `minisign` CLI is installed,
  the signatures of both, with the same trusted-comment check; `--require-signature` makes a
  missing `minisign` an error. They download over HTTPS only, run the new binary once before
  installing it, install nothing unless every check passes, and never use `sudo` or ask for
  elevation. Without `minisign` the checksum proves only that the download is intact, and the
  scripts say so.
- `cargo binstall jev-cli` downloads the same archives and checks their minisign signature against
  the primary key, which the crate's metadata names. The crates are published to crates.io by the
  release workflow alone, for stable releases, after the release assets were verified.

### Key rotation

There are always two key pairs. The **primary** key signs every release. The **next** key was
generated in advance: its public half is already published above and compiled into every `jev`,
and its secret half is not available to the release workflow and has never signed anything.

To rotate, planned or because the primary key may be compromised:

1. Promote the next key: from now on the release workflow signs with it.
2. Generate a new next key pair, and commit its public key, in `crates/jev-cli/keys` and in the
   install scripts. Put the promoted key in `[package.metadata.binstall.signing]` in
   `crates/jev-cli/Cargo.toml`, which is how `cargo binstall` checks the archives.
3. Ship a release signed with the promoted key that compiles in the promoted key as primary and
   the new key as next. Installed copies already trust the promoted key, so they accept this
   release, and from then on trust the new pair only.
4. Update the table above in the same pull request.

### If a signing key is compromised

1. Rotate immediately, as above, and publish the new release.
2. Publish a GitHub security advisory that names the compromised key id, the affected releases,
   and the new keys.
3. Delete any release that the compromised key signed and that was not built by the release
   workflow (its build provenance attestation is missing or does not verify).

If both keys could be compromised at once, installed copies cannot be moved to new keys by an
update. The advisory then says so, and asks users to reinstall by hand after verifying the new
release with the keys published in it and in this file.

## Supply chain

- `Cargo.lock` is committed and every build uses `--locked`.
- `cargo deny` gates advisories, licences, banned crates and crate sources on every pull request;
  `cargo audit` runs there too, and both run daily against new advisories.
- Every third-party GitHub Action is pinned to a full commit SHA, checked in CI.
- Each release publishes a CycloneDX SBOM and build provenance attestations, and every asset is
  signed (see [Verifying a release](#verifying-a-release)).
- Commits to `main` must be signed and arrive through a pull request.

`jev` is an unofficial project and is not affiliated with TypeSafe AI. Problems with the TypeSafe
API or platform itself should be reported to TypeSafe AI.
