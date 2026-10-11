import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { createContext, runInContext } from 'node:vm'

const source = (await readFile(fileURLToPath(new URL('../Sources/NoraPluginKit/Resources/script-api.js', import.meta.url)), 'utf8')).replace(/^export default /, '')
const requests: unknown[] = []
const context = createContext({ transport: async (method: string, params: unknown) => {
  requests.push({ method, params })
  return 'connected'
} })
const controller = runInContext(`(${source})(transport, 'zh-Hans')`, context)
assert.equal(runInContext('nora.locale', context), 'zh-Hans')
assert.equal(runInContext('Object.isFrozen(nora) && Object.isFrozen(nora.ui)', context), true)
assert.equal(runInContext('Object.getOwnPropertyDescriptor(globalThis, "nora").configurable', context), false)
assert.throws(() => runInContext('nora.registerCommand("", () => null)', context))
runInContext('nora.registerCommand("com.example.echo", async args => ({text: await nora.request("credentials.get", {key: args.key})}))', context)
assert.throws(() => runInContext('nora.registerPanel("com.example.echo", () => nora.ui.text("Duplicate"))', context))
assert.deepEqual(JSON.parse(JSON.stringify(await controller.invokeCommand('com.example.echo', { key: 'github' }))), { text: 'connected' })
assert.deepEqual(JSON.parse(JSON.stringify(requests)), [{ method: 'credentials.get', params: { key: 'github' } }])
await assert.rejects(() => controller.invokeCommand('not.registered', {}))
runInContext('nora.registerPanel("com.example.panel", event => nora.ui.column({type: "webview",children:[nora.ui.text(event.values.message ?? event.locale)]}))', context)
assert.deepEqual(JSON.parse(JSON.stringify(await controller.invokePanel('com.example.panel'))), { type: 'column', children: [{ type: 'text', text: 'zh-Hans' }] })
assert.equal((await controller.invokePanel('com.example.panel', { values: { message: 'Hello' } })).children[0].text, 'Hello')
assert.throws(() => runInContext('nora.ui.node("webview", {})', context))
console.log('Script author API registration, transport, native views, and isolation tests passed')
