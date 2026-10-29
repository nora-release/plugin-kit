#!/bin/bash
set -euo pipefail

package_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_root="$package_root/.build"
nora_plugin=(cargo run --quiet --manifest-path "$package_root/Cargo.toml" -p nora-plugin --)
staging_root="$(mktemp -d "$build_root/example.XXXXXX")"
trap 'rm -rf "$staging_root"' EXIT

"${nora_plugin[@]}" bundle --config "$package_root/Examples/plugin-release.json" --output "$staging_root" --debug
mv "$staging_root/com.nora.example.native.app" "$staging_root/NoraPluginExample.app"
printf '{"com.apple.security.app-sandbox":true}' | plutil -convert xml1 -o "$staging_root/sandbox.entitlements" -
codesign --force --sign - --options runtime --entitlements "$staging_root/sandbox.entitlements" "$staging_root/NoraPluginExample.app/Contents/Extensions/NativePlugin.appex"
codesign --force --sign - --options runtime --entitlements "$staging_root/sandbox.entitlements" "$staging_root/NoraPluginExample.app"
codesign --verify --deep --strict "$staging_root/NoraPluginExample.app"
rm -rf "$build_root/NoraPluginExample.app"
mv "$staging_root/NoraPluginExample.app" "$build_root/NoraPluginExample.app"
ditto -c -k --sequesterRsrc --keepParent "$build_root/NoraPluginExample.app" "$staging_root/NoraPluginExample.zip"
mv "$staging_root/NoraPluginExample.zip" "$build_root/NoraPluginExample.zip"

printf 'App: %s\n' "$build_root/NoraPluginExample.app"
"${nora_plugin[@]}" local-marketplace --name 'Nora Native Examples' \
    --manifest "$package_root/Examples/nora-plugin.json" \
    --archive "$build_root/NoraPluginExample.zip" --output "$build_root/marketplace.json"
