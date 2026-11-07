.DEFAULT_GOAL := check

CARGO ?= cargo

.PHONY: build test example fmt check

build:
	$(CARGO) build --workspace --all-targets

test:
	$(CARGO) test --workspace

example:
	$(CARGO) run -p nora-plugin --example agent

fmt:
	$(CARGO) fmt --all

check:
	$(CARGO) fmt --all --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	$(CARGO) test --workspace
