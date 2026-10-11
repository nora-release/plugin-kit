#!/bin/bash
set -euo pipefail

package_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_root="$package_root/.build"
swift build --package-path "$package_root" --product NoraPluginExample
swift build --package-path "$package_root" --product NoraPluginExampleHost
binary_root="$(swift build --package-path "$package_root" --show-bin-path)"
staging_root="$(mktemp -d "$build_root/example.XXXXXX")"
trap 'rm -rf "$staging_root"' EXIT

python3 - "$package_root" "$staging_root" "$binary_root" <<'PY'
import json
import pathlib
import plistlib
import shutil
import sys

package, staging, binaries = map(pathlib.Path, sys.argv[1:])
manifest_path = package / 'Examples/nora-plugin.json'
manifest = json.loads(manifest_path.read_text())
app = staging / 'NoraPluginExample.app'
extension = app / 'Contents/Extensions/NativePlugin.appex'
for bundle in (app, extension):
    (bundle / 'Contents/MacOS').mkdir(parents=True)
    (bundle / 'Contents/Resources').mkdir()

def write_plist(path, value):
    path.write_bytes(plistlib.dumps(value))

common = {
    'CFBundleDevelopmentRegion': 'en',
    'CFBundleInfoDictionaryVersion': '6.0',
    'CFBundleShortVersionString': manifest['version'],
    'CFBundleVersion': '1',
    'LSMinimumSystemVersion': '14.0',
}
write_plist(app / 'Contents/Info.plist', dict(common, **{
    'CFBundleIdentifier': manifest['id'],
    'CFBundleExecutable': 'NoraPluginExampleHost',
    'CFBundleName': manifest['name'],
    'CFBundlePackageType': 'APPL',
    'NSPrincipalClass': 'NSApplication',
}))
write_plist(extension / 'Contents/Info.plist', dict(common, **{
    'CFBundleIdentifier': manifest['extensionBundleIdentifier'],
    'CFBundleExecutable': 'NoraPluginExample',
    'CFBundleName': manifest['name'],
    'CFBundlePackageType': 'XPC!',
    'EXAppExtensionAttributes': {
        'EXExtensionPointIdentifier': 'com.nora.plugin.native',
        'EXExtensionPrincipalClass': 'NoraPluginExample.NativePlugin',
    },
}))
write_plist(staging / 'sandbox.entitlements', {'com.apple.security.app-sandbox': True})
shutil.copy2(manifest_path, app / 'Contents/Resources/nora-plugin.json')
shutil.copy2(binaries / 'NoraPluginExampleHost', app / 'Contents/MacOS/NoraPluginExampleHost')
shutil.copy2(binaries / 'NoraPluginExample', extension / 'Contents/MacOS/NoraPluginExample')
PY

codesign --force --sign - --options runtime --entitlements "$staging_root/sandbox.entitlements" "$staging_root/NoraPluginExample.app/Contents/Extensions/NativePlugin.appex"
codesign --force --sign - --options runtime --entitlements "$staging_root/sandbox.entitlements" "$staging_root/NoraPluginExample.app"
codesign --verify --deep --strict "$staging_root/NoraPluginExample.app"
rm -rf "$build_root/NoraPluginExample.app"
mv "$staging_root/NoraPluginExample.app" "$build_root/NoraPluginExample.app"
ditto -c -k --sequesterRsrc --keepParent "$build_root/NoraPluginExample.app" "$staging_root/NoraPluginExample.zip"
mv "$staging_root/NoraPluginExample.zip" "$build_root/NoraPluginExample.zip"

python3 - "$package_root" <<'PY'
import hashlib
import json
import pathlib
import sys

package = pathlib.Path(sys.argv[1])
archive = package / '.build/NoraPluginExample.zip'
marketplace = {
    'schemaVersion': 1,
    'name': 'Nora Native Examples',
    'plugins': [{
        'manifest': json.loads((package / 'Examples/nora-plugin.json').read_text()),
        'downloadURL': archive.name,
        'sha256': hashlib.file_digest(archive.open('rb'), 'sha256').hexdigest(),
    }],
}
destination = package / '.build/marketplace.json'
destination.write_text(json.dumps(marketplace, indent=2) + '\n')
print(f'App: {archive.with_suffix(".app")}')
print(f'Marketplace: {destination}')
PY
