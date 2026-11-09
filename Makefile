.DEFAULT_GOAL := check

CARGO ?= cargo

GO ?= go
SWIFT ?= swift

.PHONY: build test test-go test-swift example fmt check

build:
	$(CARGO) build --workspace --all-targets

test:
	$(CARGO) test --workspace

test-go:
	$(GO) -C sdk-go vet ./...
	$(GO) -C sdk-go test -race -timeout 60s ./...

# The Swift SDK is checked on macOS, where Nora's Swift plugins run.
test-swift:
ifeq ($(shell uname -s),Darwin)
	$(SWIFT) test --package-path sdk-swift --no-parallel
endif

example:
	$(CARGO) run -p nora-plugin --example agent

fmt:
	$(CARGO) fmt --all

check:
	$(CARGO) fmt --all --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	$(CARGO) test --workspace
	test -z "$$(gofmt -l sdk-go)"
	$(MAKE) test-go test-swift
