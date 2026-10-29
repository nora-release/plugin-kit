#!/bin/bash
set -euo pipefail

# Builds Examples/ProviderPlugin into .build/EchoExample, a folder Nora imports
# from Plugins > Import Plugin > AI Providers.
package_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output="$package_root/.build/EchoExample"
cargo build --quiet --release --manifest-path "$package_root/Cargo.toml" -p nora-example-provider
target_dir="$(cargo metadata --format-version 1 --no-deps --manifest-path "$package_root/Cargo.toml" | plutil -extract target_directory raw -o - -)"
rm -rf "$output"
mkdir -p "$output/bin"
cp -R "$package_root/Examples/ProviderPlugin/package/." "$output/"
cp "$target_dir/release/nora-example-provider" "$output/bin/nora-example-provider"
printf 'Provider plugin: %s\n' "$output"
