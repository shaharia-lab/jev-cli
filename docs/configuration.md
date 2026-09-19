# Configuration, profiles and the API key

Everything `jev` needs to make a call — a key, a base URL, a model, a timeout — can come from a
flag, from the environment, or from a profile in one configuration file. This page says where
each value comes from, how to change it, and where the key is kept.

## Where a value comes from

**flag > environment variable > profile > built-in default.** Every command resolves its settings
the same way, and `jev config get` shows the winner and where it came from:

```bash
jev config get model                 # the value, its source and its origin
jev config get model --field value   # just the value
jev config list                      # every setting, with its source
```

`jev config list` is the quickest answer to "why did that run use the wrong model?".

## The settings

| Setting | Flag | Environment | Default | What it does |
| --- | --- | --- | --- | --- |
| `base_url` | `--base-url` | `TYPESAFE_BASE_URL` | `https://api.typesafe.ai` | API root. Must be `https://`, unless it is loopback or `--insecure-allow-http` is given. |
| `model` | `--model` | `TYPESAFE_DEFAULT_MODEL` | `jev-latest` | Model alias or versioned id. Pin a versioned id for reproducible answers and a known price. |
| `output` | `--output` | `JEV_OUTPUT` | table on a terminal, json when piped | `table`, `json`, `yaml` or `jsonl`. |
| `timeout` | `--timeout` | | `30s` | Time allowed per attempt, e.g. `500ms`, `30s`, `2m`. |
| `max_retries` | `--max-retries` | | `2` | Retries after the first attempt (408, 429, 5xx and connection errors only). |
| `concurrency` | `--concurrency` | | `4` | Parallel requests in a batch run. Above roughly 8, a shared key gets rate limited. |
| `warn_unpinned` | `--warn-unpinned` | | `false` | Remind me on stderr when a request used an alias instead of a versioned model id. |
| `update.auto` | | `JEV_AUTO_UPDATE` | `true` | Update `jev` automatically in the background. |
| `update.channel` | | | `stable` | Releases to follow: `stable` or `prerelease`. |
| `update.pin_version` | | | unset | Stay on one version; this turns automatic updates off. |

Each profile has its own copy of the first seven. The `update.*` settings are shared by every
profile, because a machine has one `jev` binary.

```bash
jev config set model jev-1.13.0      # store it in the selected profile
jev config unset model               # back to the default
jev config set update.auto false     # for every profile
```

The API key is not a setting: it never goes into the configuration file. See
[The API key](#the-api-key) below.

## The configuration file

`jev config path` prints it. By default:

| Platform | Directory |
| --- | --- |
| Linux and other Unix | `$XDG_CONFIG_HOME/jev`, or `~/.config/jev` |
| macOS | `~/Library/Application Support/jev` |
| Windows | `%APPDATA%\jev` |

`JEV_CONFIG_DIR` moves the whole directory, which is how CI jobs and tests keep out of a person's
real configuration. `jev config path` works even when the file is broken, so it is the way to
find a file that needs fixing.

The file is TOML, and editing it by hand is fine — `jev config set` keeps your comments and only
writes the keys you asked for:

```toml
# jev configuration. Change it with `jev config set` and `jev profile`, or by hand.
# API keys are never stored in this file.
schema_version = 1
active_profile = "ci"

[profiles.default]
model = "jev-1.13.0"

[profiles.ci]
model = "jev-1.13.0"
output = "json"
timeout = "60s"

[update]
channel = "prerelease"
```

## Profiles

A profile is a named set of defaults plus its own stored API key: one for your own account, one
for a team key, one for a staging endpoint.

```bash
jev profile create ci --model jev-1.13.0 --output json
jev config set --profile ci timeout 60s
jev auth login --profile ci            # its own key
jev profile use ci                     # make it the one used by default
jev profile list                       # which exist, which is active
jev noul "Is this spam?" --state "$MESSAGE" --profile default   # for one run
```

`--profile` wins over `JEV_PROFILE`, which wins over the active profile set by `jev profile use`.
`jev profile delete ci` removes the profile, its settings and its stored key; `default` cannot be
deleted.

## The API key

TypeSafe authenticates with a bearer API key, and `jev` looks for it in exactly two places:

1. **`TYPESAFE_API_KEY`** in the environment. It always wins, and it is what CI jobs, containers
   and agents should use.
2. The **`credentials` file** that `jev auth login` writes next to `config.toml`. It is created
   with mode `0600` in a `0700` directory, and `jev` refuses to read it when others can, telling
   you the `chmod` to run.

There is no OS keychain, no OAuth, no SSO and no browser flow. The key is never accepted as a
flag value, so it cannot end up in your shell history or in a process list: `jev auth login`
prompts for it without echoing, or reads it from stdin with `--with-token`.

```bash
jev auth login                              # prompts, checks the key, stores it
printf '%s' "$KEY" | jev auth login --with-token
jev auth status                             # which key is in use, and whether the API accepts it
jev auth status --offline                   # without calling the API
jev auth logout                             # delete the stored key for this profile
```

Only the last four characters of a key are ever shown (`fingerprint`), and the key never appears
in output, logs, errors, `--dry-run` output or MCP results — `--debug-bodies` and `-vv` included.

In an environment without a terminal, `jev` never prompts: with stdin not a TTY, `--no-input`,
`JEV_NO_INPUT` or `CI=true` it fails with an error naming the flag or variable to use instead.

## Environment variables

| Variable | What it does |
| --- | --- |
| `TYPESAFE_API_KEY` | The API key. Wins over a stored one. |
| `TYPESAFE_BASE_URL` | API root, as `--base-url`. |
| `TYPESAFE_DEFAULT_MODEL` | Default model, as `--model`. |
| `TYPESAFE_LOG_LEVEL` | How much `jev` logs to stderr (`error`, `warn`, `info`, `debug`, `trace`), as `--verbose` does. |
| `JEV_OUTPUT` | Output format, as `--output`. |
| `JEV_PROFILE` | Profile to use, as `--profile`. |
| `JEV_NO_INPUT` | Never prompt, as `--no-input`. |
| `JEV_CONFIG_DIR` | The configuration directory, holding `config.toml` and `credentials`. |
| `JEV_AUTO_UPDATE` | `false` turns the background update check off for one environment. |
| `CI` | `true` turns prompts and background updates off. |
| `NO_COLOR` | Any value disables colour, as `--no-color`. |

The first four are the variables TypeSafe's own SDKs read, so a shell already set up for them
works with `jev` unchanged. Settings that only `jev` has use the `JEV_` prefix.

## Updates

`jev` updates itself only when the install script installed it. To see what applies now:

```bash
jev version            # version, how it was installed, and whether it updates itself
jev update --check     # exit 20 when a newer release exists; changes nothing
```

To turn automatic updates off, any one of these is enough:

```bash
jev config set update.auto false             # for good, on this machine
export JEV_AUTO_UPDATE=false                 # for one environment
jev config set update.pin_version 0.3.1      # stay on one version
```

It is already off for Homebrew and cargo installs (they are updated by their package manager),
when `CI=true`, and when `jev` cannot write to its own directory. The only host it contacts for
this is GitHub Releases of `shaharia-lab/jev-cli`; every download is verified against signing keys
compiled into the binary before anything is replaced. See
[Security and privacy](../README.md#security-and-privacy).
