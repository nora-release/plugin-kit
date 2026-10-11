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
python3 - "$app" <<'PY'
import pathlib
import plistlib
import sys
app = pathlib.Path(sys.argv[1])
info = {
    'CFBundleIdentifier': 'com.nora.plugin.smoke',
    'CFBundleExecutable': 'NoraPluginSmokeHost',
    'CFBundleName': 'Nora Plugin Smoke Host',
    'CFBundlePackageType': 'APPL',
    'CFBundleVersion': '1',
    'CFBundleShortVersionString': '1.0.0',
    'LSMinimumSystemVersion': '14.0',
    'NSPrincipalClass': 'NSApplication',
}
(app / 'Contents/Info.plist').write_bytes(plistlib.dumps(info))
definition = {'com.nora.plugin.native': {'EXPresentsUserInterface': True}}
(app / 'Contents/Extensions/com.nora.plugin.native.appextensionpoint').write_bytes(plistlib.dumps(definition))
PY
codesign --force --sign - --options runtime "$app"
codesign --verify --deep --strict "$app"
open -n -W "$app" --args "$report" "${SMOKE_HOLD_SECONDS:-0}"
python3 - "$report" <<'PY'
import json
import pathlib
import sys
path = pathlib.Path(sys.argv[1])
if not path.is_file():
    raise SystemExit('Smoke host exited without a report.')
report = json.loads(path.read_text())
print(json.dumps(report, indent=2))
raise SystemExit(0 if report['success'] else 1)
PY
