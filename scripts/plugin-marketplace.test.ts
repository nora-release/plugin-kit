import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
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
const networkManifest: PluginManifest = { ...manifest, networkDomains: ['api.github.com', 'github.com'],
  permissions: ['hostInfo', 'notifications', 'network', 'credentials', 'openExternal', 'chat'] }
assert.deepEqual(validateManifest(networkManifest), networkManifest)
assert.throws(() => validateManifest({ ...manifest, permissions: ['chat', 'chat'] }))
for (const patch of [{ runtime: 'script' }, { runtime: 'nativeExtension' }, { runtime: null }, { entryPoint: 'plugin.js' }, { entryPoint: null }, { schemaVersion: 2 }]) {
  assert.throws(() => validateManifest({ ...manifest, ...patch }))
}
for (const iconPath of ['assets/brand.svg', 'brand.png', 'images/brand.jpg', 'images/brand.jpeg']) {
  assert.equal(validateManifest({ ...manifest, iconPath }).iconPath, iconPath)
}
for (const iconPath of ['', '../brand.svg', '/brand.svg', 'a/../brand.svg', 'a/./brand.svg', 'a//brand.svg', 'a\\brand.svg', 'brand.svg\n', 'brand.js', 'brand.SVG', 'Contents/Resources/brand.svg', 'contents/resources/brand.svg', 'a/'.repeat(513) + 'brand.svg', null]) {
  assert.throws(() => validateManifest({ ...manifest, iconPath }))
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
for (const networkDomains of [['*.github.com'], ['https://github.com'], ['GitHub.com'], ['github.com:443'], ['github.com\n'], ['github.com', 'github.com']]) {
  assert.throws(() => validateManifest({ ...networkManifest, networkDomains }))
}
assert.throws(() => validateManifest({ ...networkManifest, permissions: [] }))
const nativeIconManifest: PluginManifest = { ...manifest, iconPath: 'brand.svg' }
const iconFiles = [{ name: 'Test.app/Contents/Resources/nora-plugin.json', data: Buffer.from(JSON.stringify(nativeIconManifest)) }]
assert.deepEqual(manifestFromArchive(zip([...iconFiles, { name: 'Test.app/Contents/Resources/brand.svg', data: iconSVG }])), nativeIconManifest)
for (const icon of [undefined, { data: iconSVG, mode: 0o120777 }, { data: Buffer.alloc(maximumIconBytes + 1) }, { data: Buffer.from('<svg><script/></svg>') }]) {
  assert.throws(() => manifestFromArchive(zip(icon ? [...iconFiles, { name: 'Test.app/Contents/Resources/brand.svg', ...icon }] : iconFiles)))
}
for (const name of ['TestPlugin/nora-plugin.json', 'Test.app/nora-plugin.json', 'TestPlugin/Contents/Resources/nora-plugin.json']) {
  assert.throws(() => manifestFromArchive(zip([{ name, data: Buffer.from(JSON.stringify(manifest)) }])))
}
assert.throws(() => validateEntry({ ...entry, teamIdentifier: undefined }))
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
  result = run('init-author', '--directory', join(temporary, 'author'))
  assert.equal(result.status, 0, result.stderr)
  assert.notEqual(run('init-author', '--directory', join(temporary, 'author')).status, 0)
  assert.notEqual(run('init-script-author', '--directory', join(temporary, 'rejected-script')).status, 0)
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
  result = run('init-author', '--directory', join(temporary, 'local-author'), '--sdk-path', sdkRoot)
  assert.equal(result.status, 0, result.stderr)
  assert.match(await readFile(join(temporary, 'local-author/Package.swift'), 'utf8'), /\.package\(name: "plugin-kit", path:/)
  assert.notEqual(run('init-author', '--directory', join(temporary, 'conflicting-author'), '--sdk-path', sdkRoot, '--sdk-version', '1.0.0').status, 0)
  result = run('init-author', '--directory', join(temporary, 'custom-author'), '--sdk-url', 'https://github.com/example/sdk.git', '--sdk-version', '1.2.3')
  assert.equal(result.status, 0, result.stderr)
  assert.match(await readFile(join(temporary, 'custom-author/Package.swift'), 'utf8'), /url: "https:\/\/github.com\/example\/sdk\.git", exact: "1\.2\.3"/)
  assert.match(await readFile(join(temporary, 'custom-author/Package.swift'), 'utf8'), /package: "sdk"/)
  const independentSDK = join(temporary, 'independent-sdk')
  await mkdir(independentSDK)
  for (const path of ['scripts', 'templates', 'Examples', 'Package.swift', 'package.json', 'LICENSE', 'sdk-version.json']) {
    await cp(join(sdkRoot, path), join(independentSDK, path), { recursive: true })
  }
  const copiedTool = join(independentSDK, 'scripts/plugin-marketplace.ts')
  for (const kind of ['registry', 'author']) {
    result = spawnSync(process.execPath, [copiedTool, `init-${kind}`, '--directory', join(temporary, `independent-${kind}`)], { cwd: temporary, encoding: 'utf8' })
    assert.equal(result.status, 0, `Independent SDK init-${kind}: ${result.stderr}`)
  }
} finally { await rm(temporary, { recursive: true, force: true }) }

const exampleManifest = JSON.parse(await readFile(resolve(scriptDirectory, '../Examples/nora-plugin.json'), 'utf8'))
validateManifest(exampleManifest)
console.log('Plugin marketplace validation, archive, release metadata, and standalone template tests passed')
