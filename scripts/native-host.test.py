import argparse
import importlib.util
import json
import pathlib
import plistlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parent.parent
RUN = subprocess.run


def scaffold(directory):
    RUN(['node', str(ROOT / 'scripts/plugin-marketplace.ts'), 'init-author',
         '--directory', str(directory), '--sdk-path', str(ROOT)],
        check=True, capture_output=True, text=True, timeout=30)


def package_fixture(directory):
    spec = importlib.util.spec_from_file_location('native_builder', directory / 'scripts/build-distribution.py')
    builder = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(builder)
    binaries = directory / 'binaries'
    binaries.mkdir()
    for product in ('PluginHost', 'PluginExtension'):
        (binaries / product).write_bytes(product.encode())

    def build_command(arguments, **kwargs):
        if arguments[:2] == ['swift', 'build']:
            return subprocess.CompletedProcess(arguments, 0)
        return RUN(arguments, **kwargs)

    def tool_output(*arguments):
        if arguments[:2] == ('swift', 'build') and arguments[-1] == '--show-bin-path':
            return str(binaries)
        if arguments[:2] == ('lipo', '-archs'):
            return 'arm64 x86_64'
        raise AssertionError(f'Unexpected packaging command: {arguments}')

    # Build-tool stand-ins keep this packaging check independent of SDK downloads.
    # The generated builder still validates the manifest and writes the real bundles.
    with mock.patch.object(builder.subprocess, 'run', side_effect=build_command), \
            mock.patch.object(builder, 'run', side_effect=tool_output):
        builder.build(directory / 'plugin-release.json', directory / 'dist', 'v0.1.0')
    return directory / 'dist/com.example.nora.hello.app'


def bundle_info(app):
    return plistlib.loads((app / 'Contents/Info.plist').read_bytes())


def assert_quiet_bundle(test, app):
    host = bundle_info(app)
    extension = bundle_info(app / 'Contents/Extensions/NativePlugin.appex')
    test.assertIs(host.get('LSUIElement'), True)
    test.assertNotIn('LSBackgroundOnly', host)
    test.assertEqual(host['CFBundlePackageType'], 'APPL')
    test.assertNotIn('LSUIElement', extension)
    test.assertNotIn('LSBackgroundOnly', extension)
    test.assertEqual(extension['CFBundlePackageType'], 'XPC!')
    test.assertEqual(extension['EXAppExtensionAttributes']['EXExtensionPointIdentifier'], 'com.nora.plugin.native')


class NativeHostPackagingTests(unittest.TestCase):
    def test_generated_native_scaffold_packages_only_its_containing_app_as_an_agent(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = pathlib.Path(temporary)
            scaffold(directory)
            app = package_fixture(directory)
            assert_quiet_bundle(self, app)
            self.assertEqual((app / 'Contents/MacOS/PluginHost').read_bytes(), b'PluginHost')
            self.assertEqual((app / 'Contents/Extensions/NativePlugin.appex/Contents/MacOS/PluginExtension').read_bytes(), b'PluginExtension')
            self.assertEqual(json.loads((app / 'Contents/Resources/nora-plugin.json').read_text()),
                             json.loads((directory / 'nora-plugin.json').read_text()))

    @unittest.skipUnless(sys.platform == 'darwin', 'Native host requires macOS')
    def test_generated_containing_host_typechecks_in_swift_6_without_sdk_dependencies(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = pathlib.Path(temporary)
            scaffold(directory)
            RUN(['xcrun', 'swiftc', '-typecheck', '-parse-as-library', '-swift-version', '6',
                 str(directory / 'Sources/PluginHost/PluginHost.swift')],
                check=True, capture_output=True, text=True, timeout=60)


def registration_checks(app):
    app = app.resolve(strict=True)
    expected = (ROOT / '.build/NoraPluginExample.app').resolve(strict=True)
    if app != expected:
        raise ValueError('Registration checks accept only the newly built .build/NoraPluginExample.app')
    assert_quiet_bundle(unittest.TestCase(), app)
    with tempfile.TemporaryDirectory() as temporary:
        directory = pathlib.Path(temporary)
        probe = directory / 'native-host-probe'
        RUN(['xcrun', 'swiftc', '-parse-as-library', '-swift-version', '6',
             str(ROOT / 'scripts/native-host-probe.swift'), '-o', str(probe)],
            check=True, capture_output=True, text=True, timeout=60)
        author = directory / 'author'
        scaffold(author)
        generated_app = package_fixture(author)
        RUN(['xcrun', 'swiftc', '-parse-as-library', '-swift-version', '6',
             str(author / 'Sources/PluginHost/PluginHost.swift'),
             '-o', str(generated_app / 'Contents/MacOS/PluginHost')],
            check=True, capture_output=True, text=True, timeout=60)
        for target in (app, generated_app):
            executable = target / 'Contents/MacOS' / bundle_info(target)['CFBundleExecutable']
            for mode in ('register', 'workspace', 'preview'):
                result = RUN([str(probe), str(executable), mode], capture_output=True, text=True, timeout=10)
                if result.returncode != 0:
                    raise AssertionError(f'{target.name} {mode}: {result.stdout}\n{result.stderr}')
                report = json.loads(result.stdout)
                if report['elapsedSeconds'] >= 3 or report['violations']:
                    raise AssertionError(report)
                if mode == 'register' and (report['exitStatus'] != 0 or report['visibleWindowObserved']):
                    raise AssertionError(report)
                if mode == 'preview' and not report['visibleWindowObserved']:
                    raise AssertionError(report)
                if mode == 'preview' and not report['accessoryPreviewObserved']:
                    raise AssertionError(report)
                if mode == 'workspace' and (not report['workspaceCompleted'] or not report['workspaceReturnedApplication']):
                    raise AssertionError(report)
                print(f'{target.name}: {json.dumps(report, sort_keys=True)}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--app', type=pathlib.Path,
                        help='After make example, check registration and manual preview of the new example and generated scaffold')
    arguments = parser.parse_args()
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(NativeHostPackagingTests))
    if not result.wasSuccessful():
        raise SystemExit(1)
    if arguments.app:
        registration_checks(arguments.app)
