# Local equivalents of the CI jobs. `make check` runs everything a pull request must pass.
# On Windows without make, run the cargo commands below directly.

.PHONY: check fmt fmt-check lint test msrv doc deny audit policy

MSRV := $(shell sed -n 's/^rust-version *= *"\(.*\)"/\1/p' Cargo.toml)

check: fmt-check lint test msrv doc deny audit policy

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

lint:
	cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

test:
	cargo test --workspace --all-features --locked

msrv:
	rustup toolchain install $(MSRV) --profile minimal
	cargo +$(MSRV) check --workspace --all-targets --all-features --locked

doc:
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features --locked

deny:
	cargo deny --locked check

audit:
	cargo audit --deny unsound --deny yanked

policy:
	scripts/ci/check-action-pins.sh
	scripts/ci/check-client-deps.sh
