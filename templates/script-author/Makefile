.DEFAULT_GOAL := package
NODE ?= node
PLUGIN_TAG ?= v$(shell $(NODE) -p "require('./nora-plugin.json').version")
SDK_TOOLS = node_modules/@nora/plugin-kit/scripts/plugin-marketplace.mjs

.PHONY: install check package

install:
	npm install --ignore-scripts

check:
	$(NODE) $(SDK_TOOLS) validate-manifest --manifest nora-plugin.json

package:
	$(NODE) $(SDK_TOOLS) package --config plugin-release.json --output dist --tag "$(PLUGIN_TAG)"
