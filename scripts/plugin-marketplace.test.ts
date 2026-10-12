import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { access, cp, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { deflateRawSync } from 'node:zlib'
import { downloadRelease, formatRegistry, manifestFromArchive, maximumIconBytes, validateEntry, validateIconData, validateManifest, validateRegistry, verifyArchive, verifyDownloads } from './plugin-marketplace.ts'
import type { PluginManifest, RegistryEntry } from './plugin-marketplace.ts'

const scriptDirectory = dirname(fileURLToPath(import.meta.url))
const tool = join(scriptDirectory, 'plugin-marketplace.ts')
const sdkRoot = resolve(scriptDirectory, '..')
const manifest: PluginManifest = {
  schemaVersion: 1, apiVersion: 1, id: 'com.nora.test.native', name: 'Test Plugin', version: '1.2.3',
  summary: 'A test fixture', publisher: 'Nora Tests', extensionBundleIdentifier: 'com.nora.test.native.extension',
  permissions: ['hostInfo'], panels: [{ id: 'com.nora.test.native.panel', title: 'Test', systemImage: 'puzzlepiece' }],
  commands: [{ id: 'com.nora.test.native.echo', title: 'Echo', description: 'Echo values', inputSchemaJSON: '{"type":"object"}', readOnly: true }],
}
function crc32(data: Buffer): number {
  let crc = 0xffffffff
  for (const byte of data) {
    crc ^= byte
    for (let bit = 0; bit < 8; bit += 1) crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0)
  }
  return (crc ^ 0xffffffff) >>> 0
}
function zip(entries: { name: string; data: Buffer; method?: number; mode?: number }[]): Buffer {
  const locals: Buffer[] = [], centrals: Buffer[] = []
  let offset = 0
  for (const entry of entries) {
    const name = Buffer.from(entry.name), compressed = entry.method === 0 ? entry.data : deflateRawSync(entry.data)
    const local = Buffer.alloc(30), central = Buffer.alloc(46)
    local.writeUInt32LE(0x04034b50); local.writeUInt16LE(20, 4); local.writeUInt16LE(entry.method ?? 8, 8)
    local.writeUInt32LE(crc32(entry.data), 14); local.writeUInt32LE(compressed.length, 18); local.writeUInt32LE(entry.data.length, 22); local.writeUInt16LE(name.length, 26)
    central.writeUInt32LE(0x02014b50); central.writeUInt16LE(20, 4); central.writeUInt16LE(20, 6); central.writeUInt16LE(entry.method ?? 8, 10)
    central.writeUInt32LE(crc32(entry.data), 16); central.writeUInt32LE(compressed.length, 20); central.writeUInt32LE(entry.data.length, 24); central.writeUInt16LE(name.length, 28); central.writeUInt32LE(offset, 42)
    central.writeUInt32LE(((entry.mode ?? 0o100644) << 16) >>> 0, 38)
    locals.push(local, name, compressed); centrals.push(central, name)
    offset += local.length + name.length + compressed.length
  }
  const directory = Buffer.concat(centrals), end = Buffer.alloc(22)
  end.writeUInt32LE(0x06054b50); end.writeUInt16LE(entries.length, 8); end.writeUInt16LE(entries.length, 10)
  end.writeUInt32LE(directory.length, 12); end.writeUInt32LE(offset, 16)
  return Buffer.concat([...locals, directory, end])
}
const archive = zip([{ name: 'Test.app/Contents/Resources/nora-plugin.json', data: Buffer.from(JSON.stringify(manifest)) }])
const entry: RegistryEntry = {
  manifest, downloadURL: 'https://github.com/nora-tests/plugin/releases/download/v1.2.3/plugin.zip',
  repositoryURL: 'https://github.com/nora-tests/plugin', releaseTag: 'v1.2.3', teamIdentifier: 'ABCDE12345',
  sha256: createHash('sha256').update(archive).digest('hex'),
}
assert.deepEqual(validateManifest(manifest), manifest)
for (const version of ['1.2', '01.2.3', '1.2.3-01', '1.2.3-', 'latest']) assert.throws(() => validateManifest({ ...manifest, version }))
for (const version of ['1.2.3', '1.2.3-beta.1', '1.2.3+build.2']) assert.equal(validateManifest({ ...manifest, version }).version, version)
assert.throws(() => validateManifest({ ...manifest, permissions: ['filesystem'] }))
assert.throws(() => validateManifest({ ...manifest, commands: [{ ...manifest.commands[0], id: manifest.panels[0].id }] }))
assert.throws(() => validateManifest({ ...manifest, panels: [{ ...manifest.panels[0], id: 'com.other.panel' }] }))
assert.throws(() => validateManifest({ ...manifest, commands: [{ ...manifest.commands[0], inputSchemaJSON: '{"type":"array"}' }] }))
assert.deepEqual(validateEntry(entry), entry)
const { extensionBundleIdentifier: _nativeExtension, ...scriptBase } = manifest
const scriptManifest: PluginManifest = { ...scriptBase, schemaVersion: 2, runtime: 'script', entryPoint: 'plugin.js',
  networkDomains: ['api.github.com', 'github.com'], permissions: ['network', 'credentials', 'openExternal'] }
validateManifest(scriptManifest)
assert.deepEqual(validateManifest({ ...scriptManifest, permissions: ['chat'], networkDomains: [] }).permissions, ['chat'])
assert.deepEqual(validateManifest({ ...scriptManifest, permissions: ['hostInfo', 'notifications', 'network', 'credentials', 'openExternal', 'chat'] }).permissions,
  ['hostInfo', 'notifications', 'network', 'credentials', 'openExternal', 'chat'])
assert.throws(() => validateManifest({ ...manifest, permissions: ['chat'] }))
assert.throws(() => validateManifest({ ...scriptManifest, permissions: ['chat', 'chat'], networkDomains: [] }))
for (const iconPath of ['assets/brand.svg', 'brand.png', 'images/brand.jpg', 'images/brand.jpeg']) {
  assert.equal(validateManifest({ ...scriptManifest, iconPath }).iconPath, iconPath)
  assert.equal(validateManifest({ ...manifest, iconPath }).iconPath, iconPath)
}
for (const iconPath of ['', '../brand.svg', '/brand.svg', 'a/../brand.svg', 'a/./brand.svg', 'a//brand.svg', 'a\\brand.svg', 'brand.svg\n', 'brand.js', 'brand.SVG', 'a/'.repeat(513) + 'brand.svg', null]) {
  assert.throws(() => validateManifest({ ...scriptManifest, iconPath }))
}
const iconSVG = Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M0 0h24v24H0z"/></svg>')
validateIconData(iconSVG, 'assets/brand.svg')
for (const icon of [
  '<!DOCTYPE svg [<!ENTITY x SYSTEM "file:///tmp/private">]><svg>&x;</svg>',
  '<svg><script>alert(1)</script></svg>', '<svg><image href="https://example.com/image.png"/></svg>',
  '<svg><use href="#linked"/></svg>', '<svg><foreignObject/></svg>', '<svg><path onclick="run()"/></svg>',
  '<svg><path style="fill:u&#114;l(https://example.com/x)"/></svg>', '<svg><animate attributeName="href"/></svg>',
  '<svg><path fill="u&#114;l(https://example.com/x)"/></svg>', '<svg><path></svg>', '<svg>&undefined;</svg>', '<html/>',
]) assert.throws(() => validateIconData(Buffer.from(icon), 'brand.svg'))
assert.throws(() => validateIconData(Buffer.alloc(maximumIconBytes + 1), 'brand.png'))
assert.throws(() => validateIconData(Buffer.from('not PNG'), 'brand.png'))
for (const entryPoint of ['../plugin.js', '/plugin.js', 'a//plugin.js', 'a\\plugin.js', 'plugin.swift']) {
  assert.throws(() => validateManifest({ ...scriptManifest, entryPoint }))
}
for (const networkDomains of [['*.github.com'], ['https://github.com'], ['GitHub.com'], ['github.com:443'], ['github.com\n'], ['github.com', 'github.com']]) {
  assert.throws(() => validateManifest({ ...scriptManifest, networkDomains }))
}
assert.throws(() => validateManifest({ ...scriptManifest, permissions: [] }))
assert.throws(() => validateManifest({ ...scriptManifest, schemaVersion: 1 }))
assert.throws(() => validateManifest({ ...scriptManifest, extensionBundleIdentifier: manifest.extensionBundleIdentifier }))
const scriptArchive = zip([
  { name: 'TestPlugin/nora-plugin.json', data: Buffer.from(JSON.stringify(scriptManifest)) },
  { name: 'TestPlugin/plugin.js', data: Buffer.from('nora.registerCommand("example", () => true)') },
])
const { teamIdentifier: _nativeTeam, ...scriptEntryBase } = entry
const scriptEntry: RegistryEntry = { ...scriptEntryBase, manifest: scriptManifest,
  sha256: createHash('sha256').update(scriptArchive).digest('hex') }
assert.deepEqual(validateEntry(scriptEntry), scriptEntry)
verifyArchive(scriptEntry, scriptArchive)
assert.deepEqual(manifestFromArchive(scriptArchive), scriptManifest)
const iconManifest: PluginManifest = { ...scriptManifest, iconPath: 'assets/brand.svg' }
const iconFiles = [
  { name: 'TestPlugin/nora-plugin.json', data: Buffer.from(JSON.stringify(iconManifest)) },
  { name: 'TestPlugin/plugin.js', data: Buffer.from('code') },
]
assert.deepEqual(manifestFromArchive(zip([...iconFiles, { name: 'TestPlugin/assets/brand.svg', data: iconSVG }])), iconManifest)
for (const icon of [undefined, { data: iconSVG, mode: 0o120777 }, { data: Buffer.alloc(maximumIconBytes + 1) }, { data: Buffer.from('<svg><script/></svg>') }]) {
  assert.throws(() => manifestFromArchive(zip(icon ? [...iconFiles, { name: 'TestPlugin/assets/brand.svg', ...icon }] : iconFiles)))
}
const nativeIconManifest: PluginManifest = { ...manifest, iconPath: 'Contents/Resources/brand.svg' }
assert.deepEqual(manifestFromArchive(zip([
  { name: 'Test.app/Contents/Resources/nora-plugin.json', data: Buffer.from(JSON.stringify(nativeIconManifest)) },
  { name: 'Test.app/Contents/Resources/brand.svg', data: iconSVG },
])), nativeIconManifest)
for (const files of [
  [{ name: 'TestPlugin/nora-plugin.json', data: Buffer.from(JSON.stringify(scriptManifest)) }],
  [{ name: 'Test.app/nora-plugin.json', data: Buffer.from(JSON.stringify(scriptManifest)) }, { name: 'Test.app/plugin.js', data: Buffer.from('code') }],
  [{ name: 'TestPlugin/nora-plugin.json', data: Buffer.from(JSON.stringify(scriptManifest)) }, { name: 'TestPlugin/plugin.js', data: Buffer.from('other.js'), mode: 0o120777 }],
  [{ name: 'TestPlugin/nora-plugin.json', data: Buffer.from(JSON.stringify(scriptManifest)) }, { name: 'TestPlugin/plugin.js', data: Buffer.alloc(2 * 1024 * 1024 + 1) }],
]) assert.throws(() => manifestFromArchive(zip(files)))
assert.equal(validateEntry({ ...entry, releaseTag: '1.2.3', downloadURL: entry.downloadURL.replace('/v1.2.3/', '/1.2.3/') }).releaseTag, '1.2.3')
for (const patch of [
  { releaseTag: 'latest' }, { teamIdentifier: '-' }, { sha256: '0'.repeat(64) },
  { downloadURL: 'https://github.com/other/plugin/releases/download/v1.2.3/plugin.zip' },
  { downloadURL: 'https://github.com/nora-tests/plugin/releases/latest/download/plugin.zip' },
  { downloadURL: 'http://github.com/nora-tests/plugin/releases/download/v1.2.3/plugin.zip' },
  { downloadURL: 'https://github.com/nora-tests/plugin/releases/download/v1.2.3/%2Fbad.zip' },
  { repositoryURL: 'https://user:password@github.com/nora-tests/plugin' },
]) assert.throws(() => validateEntry({ ...entry, ...patch }))
assert.throws(() => validateRegistry({ schemaVersion: 1, name: 'Tests', plugins: [entry, entry] }))
assert.deepEqual(manifestFromArchive(archive), manifest)
verifyArchive(entry, archive)
assert.throws(() => verifyArchive({ ...entry, sha256: '1'.repeat(64) }, archive))
assert.throws(() => verifyArchive({ ...entry, manifest: { ...manifest, permissions: [] } }, archive))
for (const name of ['../Test.app/Contents/Resources/nora-plugin.json', '/Test.app/Contents/Resources/nora-plugin.json', 'Test.app/../Contents/Resources/nora-plugin.json']) {
  assert.throws(() => manifestFromArchive(zip([{ name, data: Buffer.from(JSON.stringify(manifest)) }])))
}
const compressedBomb = zip([{ name: 'Test.app/Contents/Resources/nora-plugin.json', data: Buffer.alloc(513 * 1024, 32) }])
assert.throws(() => manifestFromArchive(compressedBomb))
for (const target of ['/tmp/outside', '../../../outside', 'other\\file']) {
  assert.throws(() => manifestFromArchive(zip([
    { name: 'Test.app/Contents/Resources/nora-plugin.json', data: Buffer.from(JSON.stringify(manifest)) },
    { name: 'Test.app/Contents/Resources/link', data: Buffer.from(target), mode: 0o120777 },
  ])))
}
assert.throws(() => manifestFromArchive(zip([
  { name: 'Test.app/Contents/Resources/nora-plugin.json', data: Buffer.from(JSON.stringify(manifest)) },
  { name: 'Test.app/Contents/Link', data: Buffer.from('Resources'), mode: 0o120777 },
  { name: 'Test.app/Contents/link/hidden', data: Buffer.from('payload') },
])))

let requests = 0
const fetcher: typeof fetch = async () => { requests += 1; return new Response(new Uint8Array(archive), { status: 200 }) }
const registry = validateRegistry({ schemaVersion: 1, name: 'Tests', plugins: [entry] })
await verifyDownloads(registry, undefined, fetcher)
assert.equal(requests, 1)
await verifyDownloads(registry, registry, fetcher)
assert.equal(requests, 1, 'Unchanged reviewed entries should not be downloaded again during PR checks')
let redirected = false
assert.deepEqual(await downloadRelease(entry.downloadURL, async url => {
  if (!redirected) { redirected = true; return new Response(null, { status: 302, headers: { location: 'https://release-assets.githubusercontent.com/example.zip?token=public' } }) }
  assert.equal(new URL(String(url)).hostname, 'release-assets.githubusercontent.com')
  return new Response(new Uint8Array(archive))
}), archive)
await assert.rejects(() => downloadRelease(entry.downloadURL, async () => new Response(null, { status: 302, headers: { location: 'http://127.0.0.1/private' } })))
await assert.rejects(() => downloadRelease(entry.downloadURL, async () => new Response(null, { status: 404 })))

const temporary = await mkdtemp(join(tmpdir(), 'nora-marketplace-tools-'))
try {
  const run = (...args: string[]) => spawnSync(process.execPath, [tool, ...args], { encoding: 'utf8' })
  const repository = join(temporary, 'registry')
  let result = run('init-registry', '--directory', repository)
  assert.equal(result.status, 0, result.stderr)
  const generatedTool = join(repository, 'scripts/plugin-marketplace.ts')
  assert.match(await readFile(join(repository, 'NoraPluginKit-LICENSE'), 'utf8'), /MIT License/)
  result = spawnSync(process.execPath, [generatedTool, 'validate', '--registry', join(repository, 'registry.json'), '--catalog', join(repository, 'marketplace.json'), '--verify-downloads'], { encoding: 'utf8' })
  assert.equal(result.status, 0, result.stderr)
  const workflow = await readFile(join(repository, '.github/workflows/validate-marketplace.yml'), 'utf8')
  assert.match(workflow, /pull_request:/)
  assert.doesNotMatch(workflow, /pull_request_target|secrets\.|contents: write|npm install|pnpm install/)
  assert.match(workflow, /pull_request\.base\.sha/)
  assert.notEqual(run('init-registry', '--directory', repository).status, 0, 'Existing registry must not be overwritten')
  const registryFile = join(temporary, 'registry.json'), catalogFile = join(temporary, 'marketplace.json')
  await writeFile(registryFile, JSON.stringify(registry))
  result = run('generate', '--registry', registryFile, '--output', catalogFile)
  assert.equal(result.status, 0, result.stderr)
  assert.equal(await readFile(catalogFile, 'utf8'), formatRegistry(registry))
  await writeFile(catalogFile, '{}')
  assert.notEqual(run('validate', '--registry', registryFile, '--catalog', catalogFile).status, 0)
  const manifestPath = join(temporary, 'nora-plugin.json'), archivePath = join(temporary, 'plugin.zip'), output = join(temporary, 'release')
  await writeFile(manifestPath, JSON.stringify(manifest)); await writeFile(archivePath, archive)
  result = run('release-entry', '--manifest', manifestPath, '--archive', archivePath, '--repository', entry.repositoryURL, '--tag', entry.releaseTag, '--team', entry.teamIdentifier, '--output', output)
  assert.equal(result.status, 0, result.stderr)
  const release = validateRegistry(JSON.parse(await readFile(join(output, 'marketplace.json'), 'utf8')))
  assert.deepEqual(release.plugins, [entry])
  assert.equal(await readFile(join(output, 'SHA256SUMS'), 'utf8'), `${entry.sha256}  plugin.zip\n`)
  assert.notEqual(run('release-entry', '--manifest', manifestPath, '--archive', archivePath, '--repository', entry.repositoryURL, '--tag', entry.releaseTag, '--team', entry.teamIdentifier, '--output', output).status, 0)
  const scriptAuthor = join(temporary, 'script-author')
  result = run('init-author', '--directory', scriptAuthor)
  assert.equal(result.status, 0, result.stderr)
  const scriptPublishing = await readFile(join(scriptAuthor, '.github/workflows/release-plugin.yml'), 'utf8')
  assert.doesNotMatch(scriptPublishing, /secrets\.|codesign|notarytool|xcodebuild|DEVELOPER_ID|APPLE_TEAM|--team/)
  assert.match(scriptPublishing, /ubuntu-latest/)
  const scriptPackagePath = join(scriptAuthor, 'package.json')
  const scriptPackage = JSON.parse(await readFile(scriptPackagePath, 'utf8'))
  assert.equal(scriptPackage.devDependencies['@nora/plugin-kit'], 'github:nora-release/plugin-kit#v0.5.1')
  assert.match(scriptPublishing, /npm install --ignore-scripts/)
  assert.match(scriptPublishing, /node_modules\/@nora\/plugin-kit\/scripts\/plugin-marketplace\.mjs/)
  for (const copiedSDKPath of ['scripts/plugin-marketplace.ts', 'scripts/build-script-plugin.py', 'nora.d.ts', 'Package.swift']) {
    await assert.rejects(() => access(join(scriptAuthor, copiedSDKPath)))
  }
  assert.notEqual(run('init-author', '--directory', scriptAuthor).status, 0)
  const packedSDKDirectory = join(temporary, 'packed-sdk')
  await mkdir(packedSDKDirectory)
  const packedSDK = spawnSync('npm', ['pack', '--ignore-scripts', '--json', '--pack-destination', packedSDKDirectory], { cwd: sdkRoot, encoding: 'utf8' })
  assert.equal(packedSDK.status, 0, packedSDK.stderr)
  const sdkArchive = join(packedSDKDirectory, JSON.parse(packedSDK.stdout)[0].filename)
  scriptPackage.devDependencies['@nora/plugin-kit'] = `file:${sdkArchive}`
  await writeFile(scriptPackagePath, JSON.stringify(scriptPackage))
  const installedSDK = spawnSync('npm', ['install', '--ignore-scripts', '--no-audit', '--no-fund', '--package-lock=false'], { cwd: scriptAuthor, encoding: 'utf8' })
  assert.equal(installedSDK.status, 0, installedSDK.stderr)
  const installedTool = join(scriptAuthor, 'node_modules/@nora/plugin-kit/scripts/plugin-marketplace.mjs')
  const installedGeneratedAuthor = spawnSync(process.execPath, [installedTool, 'init-author', '--directory', join(temporary, 'installed-author')], { encoding: 'utf8' })
  assert.equal(installedGeneratedAuthor.status, 0, installedGeneratedAuthor.stderr)
  const authorManifestPath = join(scriptAuthor, 'nora-plugin.json')
  const authorManifest = JSON.parse(await readFile(authorManifestPath, 'utf8'))
  authorManifest.iconPath = 'assets/brand.svg'
  await writeFile(authorManifestPath, JSON.stringify(authorManifest))
  await mkdir(join(scriptAuthor, 'assets'))
  await writeFile(join(scriptAuthor, 'assets/brand.svg'), iconSVG)
  const packaged = spawnSync('make', ['package'], { cwd: scriptAuthor, encoding: 'utf8' })
  assert.equal(packaged.status, 0, packaged.stderr)
  result = run('validate-manifest', '--manifest', join(scriptAuthor, 'nora-plugin.json'))
  assert.equal(result.status, 0, result.stderr)
  const builtScriptArchive = join(scriptAuthor, 'dist/com.example.nora.hello-0.1.0.zip')
  result = spawnSync(process.execPath, [installedTool, 'release-entry', '--manifest', join(scriptAuthor, 'nora-plugin.json'), '--archive', builtScriptArchive,
    '--repository', entry.repositoryURL, '--tag', 'v0.1.0', '--output', join(scriptAuthor, 'dist')], { encoding: 'utf8' })
  assert.equal(result.status, 0, result.stderr)
  const scriptRelease = validateRegistry(JSON.parse(await readFile(join(scriptAuthor, 'dist/marketplace.json'), 'utf8')))
  assert.equal(scriptRelease.plugins[0].manifest.runtime, 'script')
  assert.equal(scriptRelease.plugins[0].manifest.iconPath, 'assets/brand.svg')
  assert.equal('teamIdentifier' in scriptRelease.plugins[0], false)
  verifyArchive(scriptRelease.plugins[0], await readFile(builtScriptArchive))
  result = run('init-native-author', '--directory', join(temporary, 'author'))
  assert.equal(result.status, 0, result.stderr)
  const authorPackage = await readFile(join(temporary, 'author/Package.swift'), 'utf8')
  assert.match(authorPackage, /https:\/\/github.com\/nora-release\/plugin-kit\.git/)
  const sdkVersion = JSON.parse(await readFile(resolve(scriptDirectory, '../sdk-version.json'), 'utf8')).version
  assert.ok(authorPackage.includes(`exact: ${JSON.stringify(sdkVersion)}`))
  assert.match(await readFile(join(temporary, 'author/NoraPluginKit-LICENSE'), 'utf8'), /MIT License/)
  validateManifest(JSON.parse(await readFile(join(temporary, 'author/nora-plugin.json'), 'utf8')))
  assert.match(await readFile(join(temporary, 'author/Sources/PluginExtension/NativePlugin.swift'), 'utf8'), /import NoraPluginKit/)
  const publishing = await readFile(join(temporary, 'author/.github/workflows/release-plugin.yml'), 'utf8')
  assert.doesNotMatch(publishing, /pull_request|--clobber|set -x/)
  assert.match(publishing, /contents: read/)
  assert.match(publishing, /--verify-tag/)
  assert.match(publishing, /--draft=false --latest/)
  result = run('init-native-author', '--directory', join(temporary, 'local-author'), '--sdk-path', sdkRoot)
  assert.equal(result.status, 0, result.stderr)
  assert.match(await readFile(join(temporary, 'local-author/Package.swift'), 'utf8'), /\.package\(name: "plugin-kit", path:/)
  assert.notEqual(run('init-native-author', '--directory', join(temporary, 'conflicting-author'), '--sdk-path', sdkRoot, '--sdk-version', '1.0.0').status, 0)
  result = run('init-native-author', '--directory', join(temporary, 'custom-author'), '--sdk-url', 'https://github.com/example/sdk.git', '--sdk-version', '1.2.3')
  assert.equal(result.status, 0, result.stderr)
  assert.match(await readFile(join(temporary, 'custom-author/Package.swift'), 'utf8'), /url: "https:\/\/github.com\/example\/sdk\.git", exact: "1\.2\.3"/)
  assert.match(await readFile(join(temporary, 'custom-author/Package.swift'), 'utf8'), /package: "sdk"/)
  const independentSDK = join(temporary, 'independent-sdk')
  await mkdir(independentSDK)
  for (const path of ['scripts', 'templates', 'Examples', 'JavaScript', 'Package.swift', 'package.json', 'LICENSE', 'sdk-version.json']) {
    await cp(join(sdkRoot, path), join(independentSDK, path), { recursive: true })
  }
  const copiedTool = join(independentSDK, 'scripts/plugin-marketplace.ts')
  for (const kind of ['registry', 'author', 'script-author', 'native-author']) {
    result = spawnSync(process.execPath, [copiedTool, `init-${kind}`, '--directory', join(temporary, `independent-${kind}`)], { cwd: temporary, encoding: 'utf8' })
    assert.equal(result.status, 0, `Independent SDK init-${kind}: ${result.stderr}`)
  }
} finally { await rm(temporary, { recursive: true, force: true }) }

const exampleManifest = JSON.parse(await readFile(resolve(scriptDirectory, '../Examples/nora-plugin.json'), 'utf8'))
validateManifest(exampleManifest)
console.log('Plugin marketplace validation, archive, release metadata, and standalone template tests passed')
