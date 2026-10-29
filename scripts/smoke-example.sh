#!/bin/bash
set -euo pipefail

package_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
"$package_root/scripts/build-example.sh"
swift build --package-path "$package_root" --product NoraPluginSmokeHost
binary_root="$(swift build --package-path "$package_root" --show-bin-path)"
build_root="$package_root/.build"
app="$build_root/NoraPluginSmokeHost.app"
report="$build_root/native-smoke.json"
rm -rf "$app"
rm -f "$report" "$build_root/native-smoke.png"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Extensions"
cp "$binary_root/NoraPluginSmokeHost" "$app/Contents/MacOS/"
ditto "$build_root/NoraPluginExample.app/Contents/Extensions/NativePlugin.appex" "$app/Contents/Extensions/NativePlugin.appex"
printf '%s' '{"CFBundleIdentifier":"com.nora.plugin.smoke","CFBundleExecutable":"NoraPluginSmokeHost","CFBundleName":"Nora Plugin Smoke Host","CFBundlePackageType":"APPL","CFBundleVersion":"1","CFBundleShortVersionString":"1.0.0","LSMinimumSystemVersion":"14.0","NSPrincipalClass":"NSApplication"}' \
    | plutil -convert xml1 -o "$app/Contents/Info.plist" -
printf '%s' '{"com.nora.plugin.native":{"EXPresentsUserInterface":true}}' \
    | plutil -convert xml1 -o "$app/Contents/Extensions/com.nora.plugin.native.appextensionpoint" -
codesign --force --sign - --options runtime "$app"
codesign --verify --deep --strict "$app"
open -n -W "$app" --args "$report" "${SMOKE_HOLD_SECONDS:-0}"
[[ -f "$report" ]] || { printf 'Smoke host exited without a report.\n' >&2; exit 1; }
plutil -p "$report"
[[ "$(plutil -extract success raw -o - "$report")" == true ]]
