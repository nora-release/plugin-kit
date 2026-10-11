#!/usr/bin/env python3
import argparse
import json
import pathlib
import plistlib
import re
import shutil
import subprocess


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def build(config_path, destination, tag, scratch_path=None):
    config_path = pathlib.Path(config_path).resolve()
    config = json.loads(config_path.read_text())
    if config.get('schemaVersion') != 1:
        raise ValueError('Only release config schemaVersion 1 is supported')
    root = config_path.parent
    package = (root / config['packagePath']).resolve()
    manifest_path = (root / config['manifestPath']).resolve()
    manifest = json.loads(manifest_path.read_text())
    validator = pathlib.Path(__file__).with_name('plugin-marketplace.ts')
    subprocess.run(['node', str(validator), 'validate-manifest', '--manifest', str(manifest_path)], check=True)
    if tag != 'v' + manifest['version']:
        raise ValueError('Release tag must match v + the plugin manifest version')
    for key in ('hostProduct', 'extensionProduct'):
        if not re.fullmatch(r'[A-Za-z_][A-Za-z0-9_-]*', config[key]):
            raise ValueError(f'Invalid {key}')
    if not re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*', config['extensionPrincipalClass']):
        raise ValueError('extensionPrincipalClass must be Module.Class')
    minimum = config.get('minimumSystemVersion', '14.0')
    if not re.fullmatch(r'\d+\.\d+(?:\.\d+)?', minimum) or int(minimum.split('.')[0]) < 14:
        raise ValueError('minimumSystemVersion must be macOS 14.0 or later')
    destination = pathlib.Path(destination).resolve()
    app = destination / (manifest['id'] + '.app')
    if app.exists():
        raise ValueError('Refusing to overwrite an existing application')
    build_args = ['swift', 'build', '--package-path', str(package), '--configuration', 'release', '--arch', 'arm64', '--arch', 'x86_64']
    if scratch_path:
        build_args.extend(['--scratch-path', str(pathlib.Path(scratch_path).resolve())])
    for product in (config['hostProduct'], config['extensionProduct']):
        subprocess.run([*build_args, '--product', product], check=True)
    binaries = pathlib.Path(run(*build_args, '--show-bin-path').splitlines()[-1])
    extension = app / 'Contents/Extensions/NativePlugin.appex'
    for bundle in (app, extension):
        (bundle / 'Contents/MacOS').mkdir(parents=True)
        (bundle / 'Contents/Resources').mkdir()
    bundle_version = manifest['version'].split('-')[0].split('+')[0]
    common = {'CFBundleDevelopmentRegion': 'en', 'CFBundleInfoDictionaryVersion': '6.0',
              'CFBundleShortVersionString': bundle_version, 'CFBundleVersion': bundle_version,
              'LSMinimumSystemVersion': minimum}
    for bundle, product, identifier, extra in (
        (app, config['hostProduct'], manifest['id'], {'CFBundlePackageType': 'APPL', 'NSPrincipalClass': 'NSApplication'}),
        (extension, config['extensionProduct'], manifest['extensionBundleIdentifier'], {
            'CFBundlePackageType': 'XPC!', 'EXAppExtensionAttributes': {
                'EXExtensionPointIdentifier': 'com.nora.plugin.native', 'EXExtensionPrincipalClass': config['extensionPrincipalClass']}}),
    ):
        shutil.copy2(binaries / product, bundle / 'Contents/MacOS' / product)
        architectures = run('lipo', '-archs', str(bundle / 'Contents/MacOS' / product)).split()
        if not {'arm64', 'x86_64'}.issubset(architectures):
            raise ValueError(f'{product} must support arm64 and x86_64')
        (bundle / 'Contents/Info.plist').write_bytes(plistlib.dumps(dict(common, **extra,
            CFBundleIdentifier=identifier, CFBundleExecutable=product, CFBundleName=manifest['name'])))
    shutil.copy2(manifest_path, app / 'Contents/Resources/nora-plugin.json')
    for key, bundle in [('hostResources', app), ('extensionResources', extension)]:
        for relative in config.get(key, []):
            source = (root / relative).resolve()
            target = bundle / 'Contents/Resources' / source.name
            if target.exists():
                raise ValueError(f'Resource already exists: {source.name}')
            if source.is_dir():
                shutil.copytree(source, target, symlinks=True)
            else:
                shutil.copy2(source, target)
    print(f'Built universal application: {app}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--config', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--scratch-path')
    arguments = parser.parse_args()
    build(arguments.config, arguments.output, arguments.tag, arguments.scratch_path)
