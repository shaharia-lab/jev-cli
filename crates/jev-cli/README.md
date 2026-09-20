# jev-cli

Unofficial command-line tool for [TypeSafe AI](https://typesafe.ai)'s **Jev** model, built for
humans, shell scripts and AI agents. Installs a binary named `jev`.

Jev never writes text. You send it some content and typed questions, and it answers with
calibrated probabilities, so a script can branch on the answer:

```bash
jev noul "Is this customer angry?" --state-file ticket.txt --fail-under 0.7
```

Exit 0 when the answer is yes, exit 10 when it is not, and another code only when the call itself
failed.

```bash
cargo binstall jev-cli          # the signed release archive for your platform
cargo install jev-cli --locked  # or build it from source
```

Documentation: [quick start](https://github.com/shaharia-lab/jev-cli/blob/main/docs/user-guide/quick-start.md)
· [scripting and CI](https://github.com/shaharia-lab/jev-cli/blob/main/docs/user-guide/scripting-and-ci.md)
· [AI agents and MCP](https://github.com/shaharia-lab/jev-cli/blob/main/docs/user-guide/mcp.md)
· [command reference](https://github.com/shaharia-lab/jev-cli/blob/main/docs/commands.md)
· [all the docs](https://github.com/shaharia-lab/jev-cli/blob/main/docs/README.md)

Other ways to install it, including the verified install scripts and Homebrew, are in the
[project README](https://github.com/shaharia-lab/jev-cli#readme).

**Status:** before 1.0. Every command works, and flags, JSON output and exit codes may still change
in a minor release; the changelog calls it out when they do.

This project is not affiliated with, endorsed by, or sponsored by TypeSafe AI.

## Licence

Licensed under either of [Apache License, Version 2.0](https://github.com/shaharia-lab/jev-cli/blob/main/LICENSE-APACHE)
or [MIT license](https://github.com/shaharia-lab/jev-cli/blob/main/LICENSE-MIT) at your option.
