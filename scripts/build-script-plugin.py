#!/usr/bin/env python3
import argparse
import json
import pathlib
import stat
import subprocess
import unicodedata
import zipfile


MAXIMUM_FILES = 5000
MAXIMUM_BYTES = 128 * 1024 * 1024
MAXIMUM_ARCHIVE_BYTES = 256 * 1024 * 1024
MAXIMUM_SCRIPT_BYTES = 2 * 1024 * 1024


def relative_file(root, value):
    if not isinstance(value, str) or not value or '\\' in value or '\x00' in value:
        raise ValueError('Plugin paths must be relative paths inside the project')
    parts = value.split('/')
    if any(part in ('', '.', '..') for part in parts):
        raise ValueError('Plugin paths must stay inside the project')
    candidate = root
    for part in parts:
        candidate = candidate / part
        if candidate.is_symlink():
            raise ValueError('Script plugin files cannot be symbolic links')
    candidate.resolve().relative_to(root.resolve())
    return candidate


def package(config_path, destination, tag):
    config_path = pathlib.Path(config_path).resolve()
    config = json.loads(config_path.read_text())
    if config.get('schemaVersion') != 1 or config.get('runtime') != 'script':
        raise ValueError('Expected a schemaVersion 1 script release config')
    root = config_path.parent
    manifest_path = relative_file(root, config.get('manifestPath', 'nora-plugin.json'))
    validator = pathlib.Path(__file__).with_name('plugin-marketplace.mjs')
    subprocess.run(['node', str(validator), 'validate-manifest', '--manifest', str(manifest_path)], check=True)
    manifest = json.loads(manifest_path.read_text())
    if manifest.get('runtime') != 'script' or tag != 'v' + manifest['version']:
        raise ValueError('Release tag must match v + the script plugin version')
    entry = relative_file(root, manifest['entryPoint'])
    if not entry.is_file() or entry.stat().st_size > MAXIMUM_SCRIPT_BYTES:
        raise ValueError('The JavaScript entryPoint is missing or exceeds 2 MiB')
    files = {'nora-plugin.json': manifest_path, manifest['entryPoint']: entry}
    resources = config.get('resources', [])
    if not isinstance(resources, list) or len(resources) > 100:
        raise ValueError('resources must list at most 100 relative files or directories')
    for relative in resources:
        source = relative_file(root, relative)
        candidates = source.rglob('*') if source.is_dir() else [source]
        if not source.exists():
            raise ValueError('A configured plugin resource is missing')
        for candidate in candidates:
            name = candidate.relative_to(root).as_posix()
            relative_file(root, name)
            if candidate.is_dir():
                continue
            if not candidate.is_file():
                raise ValueError('Script plugin resources must be regular files')
            if name == 'nora-plugin.json' and candidate != manifest_path:
                raise ValueError('A resource cannot replace the plugin manifest')
            files[name] = candidate
    names = [unicodedata.normalize('NFC', name).lower() for name in files]
    if len(set(names)) != len(names) or len(files) > MAXIMUM_FILES:
        raise ValueError('Plugin paths conflict or exceed the file limit')
    if sum(path.stat().st_size for path in files.values()) > MAXIMUM_BYTES:
        raise ValueError('Plugin files exceed 128 MiB')
    destination = pathlib.Path(destination).resolve()
    destination.mkdir(parents=True, exist_ok=True)
    archive = destination / f"{manifest['id']}-{manifest['version']}.zip"
    if archive.exists():
        raise ValueError('Refusing to overwrite an existing plugin archive')
    try:
        with zipfile.ZipFile(archive, 'x', compression=zipfile.ZIP_DEFLATED, compresslevel=9) as output:
            for name, source in sorted(files.items()):
                item = zipfile.ZipInfo(f"{manifest['id']}/{name}")
                item.compress_type = zipfile.ZIP_DEFLATED
                item.external_attr = (stat.S_IFREG | 0o644) << 16
                output.writestr(item, source.read_bytes())
        if archive.stat().st_size > MAXIMUM_ARCHIVE_BYTES:
            raise ValueError('Plugin archive exceeds 256 MiB')
    except BaseException:
        archive.unlink(missing_ok=True)
        raise
    print(f'Archive: {archive}')
    return archive


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--config', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--tag', required=True)
    arguments = parser.parse_args()
    package(arguments.config, arguments.output, arguments.tag)
