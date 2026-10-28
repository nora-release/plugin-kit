.DEFAULT_GOAL := build

SWIFT ?= swift
CARGO ?= cargo

.PHONY: build test example smoke registration marketplace fmt

build:
	$(SWIFT) build
	$(CARGO) build --workspace

test:
	$(SWIFT) test
	$(CARGO) test --workspace --all-features

example:
	bash scripts/build-example.sh

smoke:
	bash scripts/smoke-example.sh

registration: example
	$(CARGO) test -p nora-plugin --test cli -- --ignored containing_apps_register_quietly

marketplace:
	$(CARGO) run --quiet -p nora-plugin -- $(ARGS)

fmt:
	$(CARGO) fmt --all
