import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { createContext, runInContext } from 'node:vm'

const source = (await readFile(fileURLToPath(new URL('../Sources/NoraPluginKit/Resources/script-api.js', import.meta.url)), 'utf8')).replace(/^export default /, '')
const requests: unknown[] = []
const context = createContext({ transport: async (method: string, params: unknown) => {
  requests.push({ method, params })
  return method === 'chat.openDraft' ? true : 'connected'
} })
const controller = runInContext(`(${source})(transport, 'zh-Hans')`, context)
assert.equal(runInContext('nora.locale', context), 'zh-Hans')
assert.equal(runInContext('Object.isFrozen(nora) && Object.isFrozen(nora.ui)', context), true)
assert.equal(runInContext('Object.isFrozen(nora.chat)', context), true)
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
const workspace = runInContext(`nora.ui.workspace({id:'workspace',action:'closeDetail',children:[
  nora.ui.sidebar({id:'repositories',value:'repo-1',items:[nora.ui.navigationItem({id:'repo-1',text:'Repository',action:'selectRepository',symbol:'folder',tone:'accent'})]}),
  nora.ui.column({children:[nora.ui.toolbar({children:[nora.ui.text({text:'Issues',textStyle:'headline'}),nora.ui.menu({text:'More',children:[nora.ui.button({text:'Refresh',action:'refresh',appearance:'plain'})]})]}),
    nora.ui.collection({id:'issues',value:'issue-42',action:'peekItem',items:[nora.ui.collectionItem({id:'issue-42',text:'Keyboard navigation',subtitle:'#42',symbol:'circle',tone:'success',action:'selectItem',children:[nora.ui.badge({text:'Bug',tone:'danger'}),nora.ui.text({text:'Author',textStyle:'caption',tone:'muted'})]})]})]}),
  nora.ui.column({children:[nora.ui.markdown('## Description'),nora.ui.emptyState({text:'Select an issue',subtitle:'Use arrow keys',symbol:'tray'})]})
]})`, context)
assert.equal(workspace.children[0].type, 'sidebar')
assert.equal(workspace.children[1].children[1].items[0].subtitle, '#42')
assert.equal(workspace.children[1].children[1].action, 'peekItem')
assert.equal(workspace.children[2].children[0].type, 'markdown')
for (const [property, values] of Object.entries({ textStyle: ['title', 'headline', 'body', 'caption', 'monospaced'],
  tone: ['neutral', 'accent', 'success', 'warning', 'danger', 'muted'], appearance: ['plain', 'primary', 'secondary', 'icon'] })) {
  for (const value of values) assert.equal(runInContext(`nora.ui.button({${property}:${JSON.stringify(value)}})`, context)[property], value)
  assert.throws(() => runInContext(`nora.ui.button({${property}:'arbitrary-css'})`, context))
}
runInContext('nora.registerPanel("com.example.workspace", event => nora.ui.workspace({children:[nora.ui.column({children:[nora.ui.text(event.action),nora.ui.text(event.values.issues)]})]}))', context)
for (const action of ['selectItem', 'peekItem', 'closeDetail']) {
  const result = await controller.invokePanel('com.example.workspace', { action, values: { issues: 'issue-42' } })
  assert.equal(result.children[0].children[0].text, action)
  assert.equal(result.children[0].children[1].text, 'issue-42')
}
const draft = { title: 'Issue #42', prompt: 'Help me investigate this issue.', context: 'Untrusted issue text\nwith code and metadata.', sourceURL: 'https://github.com/example/repo/issues/42' }
const requestCount = requests.length
context.draft = draft
context.draftEvent = { action: 'handoff', actionCapability: 'explicit-action-capability', values: {} }
assert.equal(await runInContext('nora.chat.openDraft(draft, draftEvent)', context), true)
assert.deepEqual(JSON.parse(JSON.stringify(requests.at(-1))), { method: 'chat.openDraft', params: { ...draft, actionCapability: 'explicit-action-capability' } })
assert.equal(requests.length, requestCount + 1)
for (const event of [undefined, {}, { action: 'handoff' }, { actionCapability: 'explicit-action-capability' },
  { action: '', actionCapability: 'explicit-action-capability' }, { action: 'handoff', actionCapability: '' },
  { action: 'handoff', actionCapability: 42 }]) {
  context.invalidEvent = event
  await assert.rejects(() => runInContext('nora.chat.openDraft(draft, invalidEvent)', context), /explicit panel action/)
}
assert.equal(requests.length, requestCount + 1, 'Missing action capabilities must not reach host transport')
runInContext(`nora.registerPanel('com.example.capability', async event => {
  const denied = await Promise.resolve().then(() => nora.chat.openDraft(draft)).then(() => false, () => true);
  await nora.chat.openDraft(draft, event);
  return nora.ui.text(String(denied));
})`, context)
const protectedView = await controller.invokePanel('com.example.capability', { action: 'handoff', actionCapability: 'another-action-capability' })
assert.equal(protectedView.text, 'true', 'An active action must not grant ambient access to a background promise')
assert.equal(requests.length, requestCount + 2)
assert.deepEqual(JSON.parse(JSON.stringify(requests.at(-1))), { method: 'chat.openDraft', params: { ...draft, actionCapability: 'another-action-capability' } })
assert.deepEqual(JSON.parse(runInContext('JSON.stringify(Object.keys(nora.chat))', context)), ['openDraft'])
assert.throws(() => runInContext('nora.chat.send(draft)', context))
assert.throws(() => runInContext('nora.chat.read()', context))
console.log('Script author API registration, transport, native views, and isolation tests passed')
