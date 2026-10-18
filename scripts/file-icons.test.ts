import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { createContext, runInContext } from 'node:vm'

const source = (await readFile(fileURLToPath(new URL('../Sources/NoraPluginKit/Resources/script-api.js', import.meta.url)), 'utf8')).replace(/^export default /, '')
const requests: unknown[] = []
const context = createContext({ transport: (...args: unknown[]) => { requests.push(args); return true } })
const controller = runInContext(`(${source})(transport)`, context)
assert.equal(runInContext('nora.features?.fileIcons === true', context), true)
assert.throws(() => runInContext('"use strict"; nora.features.fileIcons = false', context))
assert.equal(runInContext('({features:{continuousContent:true}}).features?.fileIcons === true', context), false)
assert.equal(runInContext('({}).features?.fileIcons === true', context), false)

for (const path of ['package.json', 'pnpm-lock.yaml', '.gitignore', 'src/设置/🦊.swift', 'unknown.ext',
  '/private/not-a-real-file.json', '../package.json', 'https://example.invalid/file.yaml']) {
  context.path = path
  const row = runInContext('nora.ui.itemRow({id:"file",text:"Filename",subtitle:"Nested path",fileIconPath:path,symbol:"doc",iconSVG:"<svg/>",action:"select-file",value:"selected",children:[nora.ui.badge("+1")]})', context)
  assert.equal(row.fileIconPath, path)
  assert.equal(row.symbol, 'doc')
  assert.equal(row.iconSVG, '<svg/>')
  assert.equal(row.action, 'select-file')
  assert.equal(row.value, 'selected')
  assert.equal(row.children[0].text, '+1')
}
for (const path of ['a'.repeat(4096), '界'.repeat(1365) + 'x', '🦊'.repeat(1024)]) {
  assert.equal(Buffer.byteLength(path), 4096)
  context.path = path
  assert.equal(runInContext('nora.ui.itemRow({fileIconPath:path})', context).fileIconPath, path)
  context.path = path + 'x'
  assert.throws(() => runInContext('nora.ui.itemRow({fileIconPath:path})', context), /file icon path/)
}
for (const path of ['', 42, true, [], {}, '\ud800', '\udfff', '界'.repeat(1366),
  'file\n.json', 'file\r.json', 'file\t.json', 'file\0.json', 'file\u007f.json', 'file\u0085.json',
  'file\u200b.json', 'file\u200d.json', 'file\u202e.json', 'file\u2066.json', 'file\u2028.json', 'file\u2029.json']) {
  context.path = path
  assert.throws(() => runInContext('nora.ui.itemRow({fileIconPath:path})', context), /file icon path/)
}
for (const kind of runInContext('Object.keys(nora.ui).filter(kind => !["node", "itemRow"].includes(kind))', context)) {
  context.kind = kind
  assert.throws(() => runInContext('nora.ui.node(kind,{fileIconPath:"package.json"})', context), /file icon path/)
}
assert.equal(runInContext('nora.ui.itemRow({symbol:"doc"})', context).fileIconPath, undefined)
assert.equal(runInContext('nora.ui.itemRow({fileIconPath:null,symbol:"doc"})', context).fileIconPath, null)
assert.throws(() => runInContext('nora.ui.itemRow({fileIconPath:"package.json",children:[nora.ui.button({action:"hidden"})]})', context), /static metadata/)
assert.throws(() => runInContext('nora.ui.tabBar({children:[nora.ui.button({action:"tab"})],items:[nora.ui.row({children:[nora.ui.itemRow({fileIconPath:"package.json"})]})]})', context), /static trailing|static metadata/)
const aside = runInContext('nora.ui.column({children:[nora.ui.itemRow({fileIconPath:"package.json"})]})', context)
context.aside = aside
assert.equal(runInContext('nora.ui.detail({children:[nora.ui.toolbar({}),nora.ui.column({}),nora.ui.tabBar({children:[nora.ui.button({action:"tab"})]}),nora.ui.column({})],items:[aside]})', context).items[0].children[0].fileIconPath, 'package.json')
runInContext('nora.registerPanel("com.example.files",() => nora.ui.itemRow({fileIconPath:"/private/not-a-real-file.json",symbol:"doc"}))', context)
assert.equal((await controller.invokePanel('com.example.files')).fileIconPath, '/private/not-a-real-file.json')
assert.deepEqual(requests, [], 'Rendering a file icon never requests filesystem, network, host information, or permissions')
console.log('Native file icon JavaScript contract passed')
