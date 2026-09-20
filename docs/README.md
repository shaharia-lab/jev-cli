# Documentation

## Using `jev`

| Page | Covers |
| --- | --- |
| [commands.md](commands.md) | Every command, flag, default and example — generated from the command tree, so it matches the binary. Regenerate it with `make reference`. |
| [exit-codes.md](exit-codes.md) | The exit-code contract, the gating flags, and the JSON that `jev` prints on stdout and stderr. |
| [configuration.md](configuration.md) | Settings and their precedence, the configuration file, profiles, the API key, environment variables and updates. |

The [README](../README.md) is the place to start, [SECURITY.md](../SECURITY.md) has the security
model, and [`skills/jev-cli/SKILL.md`](../skills/jev-cli/SKILL.md) is the same ground written for
an AI agent.

## Working on `jev`

| Page | Covers |
| --- | --- |
| [api-behaviour.md](api-behaviour.md) | What the TypeSafe API actually does where that differs from its documentation, plus limits, pricing and how request size is estimated. Several rules in `jev-client` exist only because of a finding here. |
| [threat-model.md](threat-model.md) | The long form of [SECURITY.md](../SECURITY.md): what is protected, which inputs are not trusted, where each protection lives and how it is tested, and the findings of the last review. Update it with any change that moves a trust boundary. |

[CONTRIBUTING.md](../CONTRIBUTING.md) explains the workflow and the quality gates, and
[CLAUDE.md](../CLAUDE.md) (to which `AGENTS.md` is a symlink) is the architecture guide for
contributors and AI agents alike.
