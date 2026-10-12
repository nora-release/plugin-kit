import importlib.util
import json
import pathlib
import subprocess
import tempfile
import unittest
import zipfile


ROOT = pathlib.Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location('script_builder', ROOT / 'scripts/build-script-plugin.py')
BUILDER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILDER)


class ScriptPluginPackagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temporary.name)
        self.config = self.root / 'plugin-release.json'
        self.config.write_text(json.dumps({'schemaVersion': 1, 'runtime': 'script', 'manifestPath': 'nora-plugin.json', 'resources': ['resources']}))
        manifest = json.loads((ROOT / 'templates/script-author/nora-plugin.json').read_text())
        (self.root / 'nora-plugin.json').write_text(json.dumps(manifest))
        (self.root / 'plugin.js').write_text('nora.registerCommand("example", () => true)')
        (self.root / 'resources').mkdir()
        (self.root / 'resources/icon.txt').write_text('resource')
        (self.root / '.env').write_text('private fixture not part of the package')

    def tearDown(self):
        self.temporary.cleanup()

    def test_packages_only_declared_code_and_resources_without_apple_binary(self):
        archive = BUILDER.package(self.config, self.root / 'dist', 'v0.1.0')
        with zipfile.ZipFile(archive) as content:
            self.assertEqual(set(content.namelist()), {
                'com.example.nora.hello/nora-plugin.json', 'com.example.nora.hello/plugin.js',
                'com.example.nora.hello/resources/icon.txt',
            })
            self.assertTrue(all(not name.endswith(('.app', '.appex')) for name in content.namelist()))
        with self.assertRaises(ValueError):
            BUILDER.package(self.config, self.root / 'dist', 'v0.1.0')

    def test_rejects_symlinked_entrypoint(self):
        (self.root / 'plugin.js').unlink()
        (self.root / 'plugin.js').symlink_to(self.root / '.env')
        with self.assertRaises(ValueError):
            BUILDER.package(self.config, self.root / 'dist', 'v0.1.0')

    def test_rejects_symlinked_resources(self):
        (self.root / 'resources/secret.txt').symlink_to(self.root / '.env')
        with self.assertRaises(ValueError):
            BUILDER.package(self.config, self.root / 'dist', 'v0.1.0')

    def icon_manifest(self):
        manifest_path = self.root / 'nora-plugin.json'
        manifest = json.loads(manifest_path.read_text())
        manifest['iconPath'] = 'assets/brand.svg'
        manifest_path.write_text(json.dumps(manifest))
        (self.root / 'assets').mkdir()
        return self.root / 'assets/brand.svg'

    def test_icon_is_packaged_even_without_a_redundant_resources_entry(self):
        self.icon_manifest().write_text('<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0h24v24H0z"/></svg>')
        archive = BUILDER.package(self.config, self.root / 'dist', 'v0.1.0')
        with zipfile.ZipFile(archive) as content:
            self.assertIn('com.example.nora.hello/assets/brand.svg', content.namelist())

    def test_rejects_missing_icon(self):
        self.icon_manifest()
        with self.assertRaises(ValueError):
            BUILDER.package(self.config, self.root / 'dist', 'v0.1.0')

    def test_rejects_symlinked_icon(self):
        self.icon_manifest().symlink_to(self.root / '.env')
        with self.assertRaises(ValueError):
            BUILDER.package(self.config, self.root / 'dist', 'v0.1.0')

    def test_rejects_linked_svg_content(self):
        self.icon_manifest().write_text('<svg><image href="https://example.com/icon.png"/></svg>')
        with self.assertRaises(subprocess.CalledProcessError):
            BUILDER.package(self.config, self.root / 'dist', 'v0.1.0')

    def test_rejects_wrong_version_tag(self):
        with self.assertRaises(ValueError):
            BUILDER.package(self.config, self.root / 'dist', 'v2.0.0')


if __name__ == '__main__':
    unittest.main()
