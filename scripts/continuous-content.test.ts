import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { createContext, runInContext } from 'node:vm'

const source = (await readFile(new URL('../Sources/NoraPluginKit/Resources/script-api.js', import.meta.url), 'utf8')).replace(/^export default /, '')
const context = createContext({ transport: async () => true })
const controller = runInContext(`(${source})(transport)`, context)
assert.equal(runInContext('nora.features.continuousContent', context), true)
runInContext('globalThis.renderCount = 0; nora.registerPanel("history", () => { renderCount++; return nora.ui.lazyColumn({children:[nora.ui.document({text:globalThis.body})]}) }); nora.registerPanel("other", () => nora.ui.text("other"))', context)
const body = '## 标题 🦉\n\n```swift\n' + 'let 中文 = "🦉"\n'.repeat(35_000) + '```\n'
context.body = body
const initial = await controller.invokePanel('history')
const transfer = initial.noraViewDocument
assert.ok(transfer.chunkCount > 1)
assert.ok(transfer.byteCount > 512 * 1024)
let serialized = ''
for (let index = 0; index < transfer.chunkCount; index++) {
  const { noraViewChunk: chunk } = await controller.invokePanel('history', { contentTransfer: { token: transfer.token, index } })
  assert.equal(chunk.token, transfer.token)
  assert.equal(chunk.index, index)
  assert.ok(Buffer.byteLength(chunk.text) <= 64 * 1024)
  assert.equal(chunk.text.isWellFormed(), true)
  assert.ok(Buffer.byteLength(JSON.stringify({ noraViewChunk: chunk })) <= 512 * 1024)
  serialized += chunk.text
}
assert.equal(Buffer.byteLength(serialized), transfer.byteCount)
assert.equal(JSON.parse(serialized).children[0].text, body)
assert.equal(runInContext('renderCount', context), 1, 'Chunk retrieval never invokes panel actions or reloads data')
for (const [panel, event] of [
  ['other', { contentTransfer: { token: transfer.token, index: 0 } }],
  ['missing', { contentTransfer: { token: transfer.token, index: 0 } }],
  ['history', { contentTransfer: { token: transfer.token, index: -1 } }],
  ['history', { contentTransfer: { token: transfer.token, index: transfer.chunkCount } }],
  ['history', { contentTransfer: { token: transfer.token, index: 0.5 } }],
  ['history', { contentTransfer: { token: transfer.token, index: 0 }, action: 'send-comment' }],
  ['history', { contentTransfer: { token: transfer.token, index: 0 }, actionCapability: 'not-authorized' }],
  ['history', { contentTransfer: null }],
]) {
  await assert.rejects(() => controller.invokePanel(panel, event), /Invalid Nora view document transfer/)
}
const second = (await controller.invokePanel('history')).noraViewDocument
await controller.invokePanel('history')
await assert.rejects(() => controller.invokePanel('history', { contentTransfer: { token: transfer.token, index: 0 } }))
assert.ok((await controller.invokePanel('history', { contentTransfer: { token: second.token, index: 0 } })).noraViewChunk)
assert.equal(runInContext('renderCount', context), 3)
for (const properties of [
  { text: '' }, { text: 'x'.repeat(2 * 1024 * 1024 + 1) }, { text: '\ud800' },
  { text: 'Text', action: 'write' }, { text: 'Text', children: [{ type: 'text' }] }, { text: 'Text', loadMoreAction: 'next', loadMoreToken: '1' },
]) {
  context.properties = properties
  assert.throws(() => runInContext('nora.ui.document(properties)', context), /Nora document|pagination/)
}
runInContext('nora.ui.lazyColumn({children:[],loadMoreAction:"next",loadMoreToken:"2"})', context)
assert.throws(() => runInContext('nora.ui.lazyColumn({children:[],loadMoreAction:"next"})', context), /pagination/)
assert.throws(() => runInContext('nora.ui.lazyColumn({items:[nora.ui.text("bad")]})', context), /Nora lazy column/)
context.body = 'small'
assert.equal((await controller.invokePanel('history')).type, 'lazyColumn')
runInContext('nora.registerPanel("oversized", () => ({type:"column",children:Array.from({length:9},()=>({type:"document",text:"x".repeat(2*1024*1024)}))}))', context)
await assert.rejects(() => controller.invokePanel('oversized'), /exceeds 16 MiB/)
console.log('continuous native content contract passed')
