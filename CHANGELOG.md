# Changelog

## 0.1.0-rc.1 (2026-09-19)


### ⚠ BREAKING CHANGES

* **cli:** drop the OS keychain; the API key comes from the environment or one private file ([#44](https://github.com/shaharia-lab/jev-cli/issues/44))

### Features

* agent skill for jev, installable with npx skills and as a Claude Code plugin ([#50](https://github.com/shaharia-lab/jev-cli/issues/50)) ([5d5632b](https://github.com/shaharia-lab/jev-cli/commit/5d5632bbeb84419f353ff55a627da9950ec2b23d))
* **cli:** `jev auth login|status|logout` with keychain storage and a private file fallback ([#43](https://github.com/shaharia-lab/jev-cli/issues/43)) ([af22039](https://github.com/shaharia-lab/jev-cli/commit/af22039c039977ac6ed754ca1a59f23f505391f6))
* **cli:** `jev batch run` over JSONL and CSV with a bounded worker pool, result records and a summary ([#47](https://github.com/shaharia-lab/jev-cli/issues/47)) ([91b7dc3](https://github.com/shaharia-lab/jev-cli/commit/91b7dc39d3208e4b3e03a3defa765a019d41bbf2))
* **cli:** `jev completion` for bash, zsh, fish and PowerShell, and man pages for every command ([#57](https://github.com/shaharia-lab/jev-cli/issues/57)) ([92e6cd0](https://github.com/shaharia-lab/jev-cli/commit/92e6cd029e5a416f46f6996a0d70c3544086781f))
* **cli:** `jev eval` and `jev models list`, with the reusable evaluation service ([#40](https://github.com/shaharia-lab/jev-cli/issues/40)) ([ef3ab50](https://github.com/shaharia-lab/jev-cli/commit/ef3ab50cce0b75cceacb41a4cb1942948e731b94))
* **cli:** `jev mcp serve`, an MCP server over stdio with evaluate, noul, choice, score, validate and list_models tools ([#45](https://github.com/shaharia-lab/jev-cli/issues/45)) ([603ab27](https://github.com/shaharia-lab/jev-cli/commit/603ab276e4740015d1326e0dae575deb9847aa3f))
* **cli:** `jev noul`, `jev choice`, `jev score` and gating exit codes ([#42](https://github.com/shaharia-lab/jev-cli/issues/42)) ([e92ab8e](https://github.com/shaharia-lab/jev-cli/commit/e92ab8e1b2615276c31210f2f821bd0ab79ac156))
* **cli:** `jev schema` prints JSON Schemas for requests, question sets, batch records, results and errors ([#48](https://github.com/shaharia-lab/jev-cli/issues/48)) ([7aea8fa](https://github.com/shaharia-lab/jev-cli/commit/7aea8fa4a4a8e13a025bc68f4d521c190202378e))
* **cli:** `jev validate`, offline request checking with human and JSON findings ([#41](https://github.com/shaharia-lab/jev-cli/issues/41)) ([0120ac9](https://github.com/shaharia-lab/jev-cli/commit/0120ac9e996a9c5db5eb9b27fee3553164361142))
* **cli:** agent-first help standard with a CI lint, and `jev spec` ([#46](https://github.com/shaharia-lab/jev-cli/issues/46)) ([002fe32](https://github.com/shaharia-lab/jev-cli/commit/002fe32f07f94d041e42cbd3e84bd86de0b32023))
* **cli:** batch resume, graceful interruption, pool-wide back-off, progress, dry-run, --limit and --ordered ([#49](https://github.com/shaharia-lab/jev-cli/issues/49)) ([9e4dc67](https://github.com/shaharia-lab/jev-cli/commit/9e4dc6773793e74faaf6fbb68b6ea49b89301bf1))
* **cli:** command tree, output layer, structured errors and the exit-code contract ([#38](https://github.com/shaharia-lab/jev-cli/issues/38)) ([e4d13e6](https://github.com/shaharia-lab/jev-cli/commit/e4d13e6bef07e75fcea62e004539355c71bcfdd4))
* **client:** HTTP transport with SDK-parity retries, typed errors and a secret API key ([#36](https://github.com/shaharia-lab/jev-cli/issues/36)) ([636a73f](https://github.com/shaharia-lab/jev-cli/commit/636a73f85de07b6413f22fd3c50111233b01d47d))
* **client:** offline request validation, lints and a calibrated token-size estimate ([#37](https://github.com/shaharia-lab/jev-cli/issues/37)) ([e27f663](https://github.com/shaharia-lab/jev-cli/commit/e27f663580ab3d0168441837e84de22d6662a1a3))
* **client:** typed request/answer model, tolerant parsing, JSON Schema and pricing ([#35](https://github.com/shaharia-lab/jev-cli/issues/35)) ([dc3c73e](https://github.com/shaharia-lab/jev-cli/commit/dc3c73e4aa945e0ef966036adc1c73e5aae61777))
* **cli:** TOML configuration, named profiles and settings with provenance ([#39](https://github.com/shaharia-lab/jev-cli/issues/39)) ([d074ce6](https://github.com/shaharia-lab/jev-cli/commit/d074ce6c89ba21a2e5a895e3517f32156dc94242))
* **mcp:** `batch_run` tool confined to --allow-dir roots with row and cost caps ([#55](https://github.com/shaharia-lab/jev-cli/issues/55)) ([5095592](https://github.com/shaharia-lab/jev-cli/commit/5095592a931002dd94486e7681f9df9b6a0e8bcb))
* **release:** publish the Homebrew formula to shaharia-lab/homebrew-tap on stable releases ([#72](https://github.com/shaharia-lab/jev-cli/issues/72)) ([9096c93](https://github.com/shaharia-lab/jev-cli/commit/9096c93ca0948577d6888be78840b888f6e6d7d3))
* **update:** jev update with verified download, atomic swap, rollback and install-method detection ([#71](https://github.com/shaharia-lab/jev-cli/issues/71)) ([daf94ef](https://github.com/shaharia-lab/jev-cli/commit/daf94ef0451dc0b32da6c66770eeb67f140639c2))


### Bug Fixes

* **cli:** batch run writes periodic progress lines to a non-terminal stderr by default ([#51](https://github.com/shaharia-lab/jev-cli/issues/51)) ([c1abd1b](https://github.com/shaharia-lab/jev-cli/commit/c1abd1b6ccdae510486cbe3d37e98801be3a6c6d))
* **cli:** config lock waits while other writers make progress, with jittered back-off ([#56](https://github.com/shaharia-lab/jev-cli/issues/56)) ([a119570](https://github.com/shaharia-lab/jev-cli/commit/a11957079994df4142cf0dec1825bcdc668a4dfd))
* **client:** escape-option lint matches common phrasings, not only exact names ([#59](https://github.com/shaharia-lab/jev-cli/issues/59)) ([da57359](https://github.com/shaharia-lab/jev-cli/commit/da57359be4309e43a07c5b4a3a09004c2c9d60bf))
* **release:** skip the Homebrew formula check for a pre-release ([#74](https://github.com/shaharia-lab/jev-cli/issues/74)) ([2affd4d](https://github.com/shaharia-lab/jev-cli/commit/2affd4dfce0cd643ba0f5fb146ed51ab7946127b))


### Documentation

* README status reflects working commands and pre-releases ([#73](https://github.com/shaharia-lab/jev-cli/issues/73)) ([c49c4b1](https://github.com/shaharia-lab/jev-cli/commit/c49c4b153b33c75b7b9483178b6a66f64b1cdbe3))


### Code Refactoring

* **cli:** drop the OS keychain; the API key comes from the environment or one private file ([#44](https://github.com/shaharia-lab/jev-cli/issues/44)) ([67d0a70](https://github.com/shaharia-lab/jev-cli/commit/67d0a7068c3adc05858a57ede0f0f78a87496090))
