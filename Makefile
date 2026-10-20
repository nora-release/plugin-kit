.DEFAULT_GOAL := build

SWIFT ?= swift
NODE ?= node

.PHONY: build test example smoke marketplace

build:
	$(SWIFT) build

test:
	$(SWIFT) test
	$(NODE) scripts/plugin-marketplace.test.ts
	PYTHONDONTWRITEBYTECODE=1 python3 scripts/native-host.test.py

example:
	bash scripts/build-example.sh

smoke:
	bash scripts/smoke-example.sh

marketplace:
	$(NODE) scripts/plugin-marketplace.ts $(ARGS)
