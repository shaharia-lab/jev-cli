# Local equivalents of the CI jobs. `make check` runs everything a pull request must pass.
# On Windows without make, run the cargo commands below directly.

.PHONY: check hooks fmt fmt-check lint test msrv doc package deny audit policy bench schemas dist-assets

MSRV := $(shell sed -n 's/^rust-version *= *"\(.*\)"/\1/p' Cargo.toml)

check: fmt-check lint test msrv doc package deny audit policy

# Once per clone: installs the pre-commit and pre-push git hooks.
hooks:
	pre-commit install

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

lint:
	cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
	cargo clippy -p jev-cli --all-targets --locked -- -D warnings  # the configuration that ships

test:
	cargo test --workspace --all-features --locked
	cargo test -p jev-cli --locked  # the configuration that ships: no internal test hooks

msrv:
	rustup toolchain install $(MSRV) --profile minimal
	cargo +$(MSRV) check --workspace --all-targets --all-features --locked

doc:
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features --locked

# Both crates exactly as crates.io would receive them, jev-cli built against the packaged
# jev-client. Uploads nothing.
package:
	cargo publish --workspace --dry-run --locked

deny:
	cargo deny --locked check

audit:
	cargo audit --deny unsound --deny yanked

policy:
	scripts/ci/check-action-pins.sh
	scripts/ci/check-client-deps.sh
	scripts/ci/check-pr-title.sh --self-test
	scripts/release/self-test.sh  # the signing round trip needs minisign; skipped without it
	scripts/release/homebrew-self-test.sh
	scripts/release/crates-self-test.sh
	scripts/ci/lint-install-scripts.sh  # ShellCheck and PSScriptAnalyzer; skipped when missing
	SKIP=no-commit-to-branch,cargo-fmt,cargo-clippy pre-commit run --all-files

# The performance budgets (PRD NFR-PERF-1..3) on a release build, one test at a time: CLI overhead,
# commands that need no network, and batch memory over a million rows (Linux only, a few minutes).
# Not part of `check`; CI runs it in the `performance` job.
bench:
	cargo test --release --locked -p jev-cli --test performance --test batch -- --ignored --nocapture --test-threads=1

# Regenerates the published JSON Schemas in schemas/ from this build. A test fails when they are
# out of date, and releases attach them for editor integration.
schemas:
	for name in request questions batch-record output error; do \
		cargo run --quiet --locked -p jev-cli -- schema $$name -o json > schemas/$$name.schema.json || exit 1; \
	done

# Man pages and shell completion scripts for a release archive or a package, generated from the
# command tree on the build host (so also for cross-compiled targets). DIR defaults to
# target/dist-assets.
DIR ?= target/dist-assets
dist-assets:
	cargo run -p jev-cli --example dist-assets --locked -- $(DIR)
