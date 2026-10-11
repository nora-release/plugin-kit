#!/bin/bash
set -euo pipefail

# Prepare distribution assets only; the release workflow publishes after validation.
if [[ $# != 5 ]]; then
    printf 'Usage: %s APPLICATION.app OUTPUT_DIRECTORY GITHUB_REPOSITORY_URL RELEASE_TAG TEAM_ID\n' "$0" >&2
    exit 2
fi
plugin_app="$1"
plugin_output="$2"
plugin_repository="$3"
plugin_tag="$4"
plugin_team="$5"
: "${NORA_PLUGIN_SIGNING_IDENTITY:?Set a Developer ID Application signing identity}"
: "${NORA_PLUGIN_KEYCHAIN_PATH:?Set the temporary signing keychain path}"
: "${NORA_PLUGIN_NOTARY_PROFILE:?Set the notarization keychain profile}"
[[ "$NORA_PLUGIN_SIGNING_IDENTITY" == 'Developer ID Application: '* ]] || { printf 'A Developer ID Application identity is required.\n' >&2; exit 1; }
[[ "$plugin_team" =~ ^[A-Z0-9]{10}$ ]] || { printf 'An Apple Developer Team ID is required.\n' >&2; exit 1; }
plugin_script_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
plugin_validator="$plugin_script_root/plugin-marketplace.ts"
plugin_manifest="$plugin_app/Contents/Resources/nora-plugin.json"
node "$plugin_validator" validate-manifest --manifest "$plugin_manifest"
plugin_stage="$(mktemp -d "${TMPDIR:-/tmp}/nora-plugin-sign.XXXXXX")"
trap 'rm -rf "$plugin_stage"' EXIT
python3 - "$plugin_stage/entitlements.plist" <<'PY'
import pathlib, plistlib, sys
pathlib.Path(sys.argv[1]).write_bytes(plistlib.dumps({'com.apple.security.app-sandbox': True}))
PY
plugin_extension="$plugin_app/Contents/Extensions/NativePlugin.appex"
[[ -d "$plugin_extension" ]] || { printf 'The application is missing its native extension.\n' >&2; exit 1; }
for plugin_bundle in "$plugin_extension" "$plugin_app"; do
    codesign --force --sign "$NORA_PLUGIN_SIGNING_IDENTITY" --keychain "$NORA_PLUGIN_KEYCHAIN_PATH" \
        --options runtime --timestamp --entitlements "$plugin_stage/entitlements.plist" "$plugin_bundle"
    codesign --verify --deep --strict "$plugin_bundle"
    codesign -dv --verbose=4 "$plugin_bundle" 2> "$plugin_stage/signature.txt"
    if ! /usr/bin/grep -Fxq "TeamIdentifier=$plugin_team" "$plugin_stage/signature.txt"; then
        printf 'The signing certificate does not match the declared Team ID.\n' >&2
        exit 1
    fi
done
ditto -c -k --keepParent "$plugin_app" "$plugin_stage/notarization.zip"
xcrun notarytool submit "$plugin_stage/notarization.zip" --keychain-profile "$NORA_PLUGIN_NOTARY_PROFILE" \
    --keychain "$NORA_PLUGIN_KEYCHAIN_PATH" --wait --timeout 30m --output-format json > "$plugin_stage/notarization.json"
python3 - "$plugin_stage/notarization.json" <<'PY'
import json, pathlib, sys
report = json.loads(pathlib.Path(sys.argv[1]).read_text())
if report.get('status') != 'Accepted':
    raise SystemExit('Apple notarization was not accepted; no distribution assets were created.')
PY
xcrun stapler staple "$plugin_app"
xcrun stapler validate "$plugin_app"
codesign --verify --deep --strict "$plugin_app"
spctl --assess --type execute "$plugin_app"
plugin_archive_name="$(python3 - "$plugin_manifest" <<'PY'
import json, pathlib, re, sys
manifest = json.loads(pathlib.Path(sys.argv[1]).read_text())
print(manifest['id'] + '-' + re.sub('[^A-Za-z0-9._-]', '-', manifest['version']) + '-macos-universal.zip')
PY
)"
mkdir -p "$plugin_output"
[[ ! -e "$plugin_output/$plugin_archive_name" ]] || { printf 'Refusing to overwrite a release archive.\n' >&2; exit 1; }
ditto -c -k --keepParent "$plugin_app" "$plugin_output/$plugin_archive_name"
node "$plugin_validator" release-entry --manifest "$plugin_manifest" --archive "$plugin_output/$plugin_archive_name" \
    --repository "$plugin_repository" --tag "$plugin_tag" --team "$plugin_team" --output "$plugin_output"
