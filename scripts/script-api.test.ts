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
const editorRequests = requests.length
const multilineDraft = 'First line\n\n第二行\r\n\t```swift\nlet value = 1\n```\n'
context.multilineDraft = multilineDraft
const editor = runInContext('nora.ui.textEditor({id:"draft",text:"Comment",placeholder:"Write a comment",value:multilineDraft,disabled:false})', context)
assert.deepEqual(JSON.parse(JSON.stringify(editor)), {
  type: 'textEditor', id: 'draft', text: 'Comment', placeholder: 'Write a comment', value: multilineDraft, disabled: false,
})
assert.equal(editor.action, undefined)
runInContext('nora.registerPanel("com.example.editor", event => nora.ui.textEditor({id:"draft",value:event.values.draft ?? ""}))', context)
assert.equal((await controller.invokePanel('com.example.editor', { values: { draft: multilineDraft } })).value, multilineDraft)
for (const value of ['', 'x'.repeat(64 * 1024), '界'.repeat(21_845) + '\n']) {
  context.editorValue = value
  assert.equal(runInContext('nora.ui.textEditor({value:editorValue})', context).value, value)
}
for (const value of ['x'.repeat(64 * 1024 + 1), '界'.repeat(21_846), 42, true, [], {}, '\ud800']) {
  context.editorValue = value
  assert.throws(() => runInContext('nora.ui.textEditor({value:editorValue})', context), /64 KiB UTF-8/)
}
assert.equal(runInContext('nora.ui.textEditor({})', context).value, undefined)
assert.equal(requests.length, editorRequests, 'Editing multiline descriptors never submits an action or contacts the host')
const diffRequests = requests.length
const rawPatch = '@@ -1,2 +1,2 @@\n-let value = 1\n+let value = 2\n // Unchanged\n'
context.rawPatch = rawPatch
for (const value of [undefined, null, 'unified', 'split']) {
  context.diffLayout = value
  const diff = runInContext('nora.ui.diff({id:"selected-file",text:rawPatch,subtitle:"Sources/示例.swift",value:diffLayout,disabled:true})', context)
  assert.equal(diff.type, 'diff')
  assert.equal(diff.text, rawPatch)
  assert.equal(diff.subtitle, 'Sources/示例.swift')
  assert.equal(diff.value, value)
  assert.equal(diff.action, undefined)
}
for (const properties of [
  { text: '界'.repeat(21_845) + '\n', subtitle: '界'.repeat(1365) + 'x' },
  { text: rawPatch, subtitle: '../Sources/main.swift', children: [], items: [], options: [] },
  { text: rawPatch, subtitle: '/tmp/main.swift' },
]) {
  context.diffProperties = properties
  assert.equal(runInContext('nora.ui.diff(diffProperties)', context).text, properties.text)
}
for (const fields of [
  { text: null }, { text: '' }, { text: 42 }, { text: 'x'.repeat(64 * 1024 + 1) }, { text: '界'.repeat(21_846) }, { text: '\ud800' },
  { subtitle: null }, { subtitle: '' }, { subtitle: 42 }, { subtitle: 'x'.repeat(4097) }, { subtitle: '界'.repeat(1366) },
  ...['\n--- /etc/passwd', '\r+++ /etc/passwd', '\u0000', '\u0085', '\u2028', '\u2029', '\u202e', '\t', '\ud800'].map(control => ({ subtitle: `file${control}.swift` })),
  { subtitle: '  ' }, { value: '' }, { value: 'detail' }, { value: 42 },
  { action: 'writeFile' }, { action: '' }, { placeholder: 'Edit' }, { symbol: 'folder' },
  { textStyle: 'body' }, { tone: 'accent' }, { appearance: 'plain' },
  { children: [{ type: 'button', action: 'writeFile' }] }, { children: {} },
  { items: [{ type: 'textField', id: 'input' }] }, { items: {} },
  { options: [{ value: 'edit', text: 'Edit' }] }, { options: {} },
]) {
  context.diffProperties = { text: rawPatch, subtitle: 'main.swift', ...fields }
  assert.throws(() => runInContext('nora.ui.diff(diffProperties)', context), /Nora diff/)
}
assert.throws(() => runInContext('nora.ui.diff()', context), /Nora diff/)
assert.throws(() => runInContext('nora.ui.node("diff",{text:rawPatch,subtitle:"main.swift"},[nora.ui.button({action:"writeFile"})])', context), /Nora diff/)
assert.equal(requests.length, diffRequests, 'Read-only patches and path-shaped filename hints never contact the host or load local files')
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
const detailRequests = requests.length
const detail = runInContext(`nora.ui.detail({id:'detail',children:[
  nora.ui.toolbar({children:[nora.ui.text('example/repo · PR #42')]}),
  nora.ui.column({children:[nora.ui.text({text:'Fix keyboard navigation',textStyle:'title'})]}),
  nora.ui.tabBar({children:[
    nora.ui.button({id:'discussion',text:'Discussion',action:'showDiscussion',value:'selected',subtitle:'4'}),
    nora.ui.button({id:'checks',text:'Checks',action:'showChecks',disabled:true,subtitle:'8'})
  ]}),
  nora.ui.splitView({text:'Back to checks',action:'back',value:'detail',children:[
    nora.ui.column({children:[nora.ui.itemRow({id:'build',text:'Build',subtitle:'macOS',symbol:'checkmark.circle',tone:'success',
      action:'selectCheck',value:'selected',children:[nora.ui.badge({text:'2m 14s'})]})]}),
    nora.ui.column({children:[nora.ui.itemRow({text:'Install dependencies',symbol:'checkmark',tone:'success',
      children:[nora.ui.row({children:[nora.ui.text({text:'14s',textStyle:'caption'}),nora.ui.badge({text:'Passed'})]})]})]})
  ]})
]})`, context)
assert.equal(detail.type, 'detail')
assert.deepEqual(JSON.parse(JSON.stringify(detail.children[2].children[0])), {
  type: 'button', id: 'discussion', text: 'Discussion', action: 'showDiscussion', value: 'selected', subtitle: '4',
})
assert.equal(detail.children[2].children[1].disabled, true)
assert.equal(detail.children[3].value, 'detail')
assert.equal(detail.children[3].children[0].children[0].action, 'selectCheck')
assert.equal(detail.children[3].children[1].children[0].action, undefined, 'Read-only steps do not acquire implicit actions')
for (const value of [undefined, null, 'list', 'detail']) {
  context.compactValue = value
  assert.equal(runInContext('nora.ui.splitView({value:compactValue,children:[nora.ui.column(),nora.ui.column()]})', context).value, value)
}
for (const expression of [
  'nora.ui.detail()',
  'nora.ui.detail({children:[nora.ui.toolbar(),nora.ui.column(),nora.ui.tabBar({children:[nora.ui.button()]})]})',
  'nora.ui.detail({children:[nora.ui.column(),nora.ui.toolbar(),nora.ui.tabBar({children:[nora.ui.button()]}),nora.ui.column()]})',
  'nora.ui.detail({children:[nora.ui.toolbar(),nora.ui.text("Heading"),nora.ui.tabBar({children:[nora.ui.button()]}),nora.ui.column()]})',
  'nora.ui.detail({children:[nora.ui.toolbar(),nora.ui.column(),nora.ui.row(),nora.ui.column()]})',
  'nora.ui.tabBar()',
  'nora.ui.tabBar({children:[nora.ui.text("Discussion")]})',
  'nora.ui.tabBar({children:[nora.ui.button({value:"arbitrary"})]})',
  'nora.ui.tabBar({children:[nora.ui.button({value:"selected"}),nora.ui.button({value:"selected"})]})',
  'nora.ui.splitView()',
  'nora.ui.splitView({children:[nora.ui.column()]})',
  'nora.ui.splitView({children:[nora.ui.column(),nora.ui.column(),nora.ui.column()]})',
  'nora.ui.splitView({value:"arbitrary",children:[nora.ui.column(),nora.ui.column()]})',
  'nora.ui.itemRow({value:"arbitrary"})',
]) assert.throws(() => runInContext(expression, context), /Nora (detail|tab bar|split view|item row)/)
for (const kind of ['detail', 'tabBar', 'splitView', 'itemRow']) {
  context.detailKind = kind
  for (const children of [42, true, {}, 'text', [null], [42], [{ type: 'webview' }]]) {
    context.invalidChildren = children
    assert.throws(() => runInContext('nora.ui.node(detailKind,{children:invalidChildren})', context), /child arrays/)
  }
  assert.throws(() => runInContext('nora.ui.node(detailKind,{items:[nora.ui.text("Dropped")]})', context), /do not accept items/)
}
for (const action of ['', ' \n', 42, true, [], {}, '界'.repeat(86), '\ud800']) {
  context.invalidRowAction = action
  for (const expression of [
    'nora.ui.itemRow({action:invalidRowAction})',
    'nora.ui.splitView({action:invalidRowAction,children:[nora.ui.column(),nora.ui.column()]})',
    'nora.ui.tabBar({children:[nora.ui.button({action:invalidRowAction})]})',
  ]) assert.throws(() => runInContext(expression, context), /native row action/)
}
assert.equal(runInContext('nora.ui.itemRow({action:"界".repeat(85)+"x"})', context).action.length, 86)
for (const expression of [
  'nora.ui.button({action:"post"})', 'nora.ui.textField({id:"hiddenInput"})', 'nora.ui.menu()',
  'nora.ui.itemRow({action:"nestedRow"})', 'nora.ui.text({text:"Hidden",action:"hiddenAction"})',
  'nora.ui.row({children:[nora.ui.badge({text:"Hidden",action:"hiddenBadgeAction"})]})',
  'nora.ui.column({items:[nora.ui.text("Hidden")]})',
  'nora.ui.text({options:[{value:"hidden",text:"Hidden"}]})',
]) assert.throws(() => runInContext(`nora.ui.itemRow({children:[${expression}]})`, context), /static metadata/)
assert.equal(requests.length, detailRequests, 'Native detail, tab, and row descriptors never invoke their actions during construction')
const paginationRequests = requests.length
const paginatedSidebar = runInContext(`nora.ui.sidebar({
  id:'items',loadMoreAction:'nextPage',loadMoreToken:'issues:all:2',items:[
    nora.ui.navigationItem({id:'issue-42',text:'Keyboard navigation',children:[nora.ui.badge({text:'Open',tone:'success'})]})
  ]
})`, context)
assert.equal(paginatedSidebar.loadMoreAction, 'nextPage')
assert.equal(paginatedSidebar.loadMoreToken, 'issues:all:2')
assert.equal(paginatedSidebar.items[0].children[0].type, 'badge')
assert.equal(runInContext('nora.ui.sidebar({id:"legacy"})', context).loadMoreAction, undefined)
assert.equal(runInContext('nora.ui.sidebar({loadMoreAction:null,loadMoreToken:null})', context).loadMoreToken, null)
for (const value of ['x'.repeat(256), 'é'.repeat(128), '界'.repeat(85) + 'x', '😀'.repeat(64)]) {
  assert.equal(Buffer.byteLength(value, 'utf8'), 256)
  context.paginationValue = value
  const sidebar = runInContext('nora.ui.sidebar({loadMoreAction:paginationValue,loadMoreToken:paginationValue})', context)
  assert.equal(sidebar.loadMoreToken, value)
}
for (const fields of [
  { loadMoreAction: 'next' }, { loadMoreToken: '2' }, { loadMoreAction: null, loadMoreToken: '2' },
  { loadMoreAction: 'next', loadMoreToken: null }, { loadMoreAction: '', loadMoreToken: '2' },
  { loadMoreAction: 'next', loadMoreToken: '' },
]) {
  context.paginationFields = fields
  assert.throws(() => runInContext('nora.ui.sidebar(paginationFields)', context), /sidebar pagination/)
}
for (const value of ['', 42, true, [], {}, 'x'.repeat(257), 'é'.repeat(129), '界'.repeat(86), '😀'.repeat(65), '\ud800', '\udfff']) {
  context.paginationValue = value
  assert.throws(() => runInContext('nora.ui.sidebar({loadMoreAction:paginationValue,loadMoreToken:"2"})', context), /sidebar pagination/)
  assert.throws(() => runInContext('nora.ui.sidebar({loadMoreAction:"next",loadMoreToken:paginationValue})', context), /sidebar pagination/)
}
const viewKinds = runInContext('Object.keys(nora.ui).filter(kind => kind !== "node" && kind !== "sidebar")', context)
for (const kind of viewKinds) {
  context.paginationKind = kind
  assert.throws(() => runInContext('nora.ui.node(paginationKind, {loadMoreAction:"next",loadMoreToken:"2"})', context), /sidebar pagination/)
}
assert.equal(requests.length, paginationRequests, 'Pagination descriptors do not send requests without a native scroll event')
const glyphRequests = requests.length
const glyphs = [
  '<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24"><!-- Icon from Huge Icons by Hugeicons - undefined --><g fill="none" stroke="currentColor" stroke-width="1.5"><path stroke-linecap="round" stroke-linejoin="round" d="M6 8v8m12 0v-4c0-2.828 0-4.243-.879-5.121C16.243 6 14.828 6 12 6h-1m0 0c0-.7 1.994-2.008 2.5-2.5M11 6c0 .7 1.994 2.008 2.5 2.5"/><circle cx="6" cy="18" r="2"/><circle cx="6" cy="6" r="2"/><circle cx="18" cy="18" r="2"/></g></svg>',
  '<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24"><!-- Icon from Huge Icons by Hugeicons - undefined --><g fill="none" stroke="currentColor" stroke-width="1.5"><path stroke-linecap="round" stroke-linejoin="round" d="M6 8v8"/><circle cx="6" cy="18" r="2"/><circle cx="6" cy="6" r="2"/><circle cx="18" cy="18" r="2"/><path stroke-linecap="round" d="M18.125 11H18m.25 0a.25.25 0 1 1-.5 0a.25.25 0 0 1 .5 0Zm-.125-5H18m.25 0a.25.25 0 1 1-.5 0a.25.25 0 0 1 .5 0Z"/></g></svg>',
  '<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24"><!-- Icon from Huge Icons by Hugeicons - undefined --><g fill="none" stroke="currentColor" stroke-width="1.5"><path stroke-linecap="round" stroke-linejoin="round" d="M6 8v8m12-5v5"/><circle cx="6" cy="18" r="2"/><circle cx="6" cy="6" r="2"/><circle cx="18" cy="18" r="2"/><path stroke-linecap="round" stroke-linejoin="round" d="m20 4l-2 2m0 0l-2 2m2-2l2 2m-2-2l-2-2"/></g></svg>',
]
for (const glyph of glyphs) {
  context.badgeGlyph = glyph
  const badge = runInContext('nora.ui.badge({text:"Status",iconSVG:badgeGlyph,symbol:"arrow.triangle.pull",tone:"success"})', context)
  assert.equal(badge.iconSVG, glyph)
  assert.equal(badge.symbol, 'arrow.triangle.pull')
}
assert.equal(runInContext('nora.ui.badge({text:"Legacy"})', context).iconSVG, undefined)
assert.equal(runInContext('nora.ui.badge({iconSVG:null})', context).iconSVG, null)
for (const value of ['x'.repeat(16 * 1024), '界'.repeat(Math.floor(16 * 1024 / 3)) + 'x']) {
  assert.equal(Buffer.byteLength(value, 'utf8'), 16 * 1024)
  context.badgeGlyph = value
  assert.equal(runInContext('nora.ui.badge({iconSVG:badgeGlyph})', context).iconSVG, value)
}
for (const value of ['', 42, true, [], {}, 'x'.repeat(16 * 1024 + 1), '界'.repeat(Math.floor(16 * 1024 / 3) + 1), '\ud800', '\udfff']) {
  context.badgeGlyph = value
  assert.throws(() => runInContext('nora.ui.badge({iconSVG:badgeGlyph})', context), /icon SVG/)
}
for (const kind of runInContext('Object.keys(nora.ui).filter(kind => kind !== "node" && kind !== "badge")', context)) {
  context.glyphKind = kind
  assert.throws(() => runInContext('nora.ui.node(glyphKind, {iconSVG:"<svg/>"})', context), /icon SVG/)
}
assert.equal(requests.length, glyphRequests, 'Inline SVG glyphs never load resources or reach host transport')
for (const [property, values] of Object.entries({ textStyle: ['title', 'headline', 'body', 'caption', 'monospaced'],
  tone: ['neutral', 'accent', 'success', 'warning', 'danger', 'muted'], appearance: ['plain', 'primary', 'secondary', 'icon'] })) {
  for (const value of values) assert.equal(runInContext(`nora.ui.button({${property}:${JSON.stringify(value)}})`, context)[property], value)
  assert.throws(() => runInContext(`nora.ui.button({${property}:'arbitrary-css'})`, context))
}
const avatarRequestCount = requests.length
const maximumAvatarBytes = 96 * 1024
for (const type of ['menu', 'button', 'navigationItem']) {
  for (const size of [1, 2, 3, maximumAvatarBytes - 2, maximumAvatarBytes - 1, maximumAvatarBytes]) {
    const avatar = Buffer.alloc(size, 255).toString('base64')
    context.avatar = avatar
    context.avatarKind = type
    const view = runInContext('nora.ui.node(avatarKind, {text:"Account", avatarBase64:avatar})', context)
    assert.equal(view.avatarBase64, avatar)
    assert.equal(view.text, 'Account')
    assert.equal(runInContext(`nora.ui.${type}({avatarBase64:avatar})`, context).avatarBase64, avatar)
  }
}
for (const avatar of ['', ' ', 'data:image/png;base64,AA==', 'https://example.com/avatar.png', 'AA==\n', 'A A==', 'AA\t==', '界',
  'AA', 'AAA', 'A===', '====', 'AAAA=', 'AA===', 'AA-_', 'AA==AAAA', 'Zh==', 'Zm9=', 42, true, [], {},
  Buffer.alloc(maximumAvatarBytes + 1).toString('base64'), 'A'.repeat(128 * 1024 + 1)]) {
  context.invalidAvatar = avatar
  assert.throws(() => runInContext('nora.ui.menu({avatarBase64:invalidAvatar})', context), /bounded standard Base64/)
}
for (const type of ['text', 'column', 'sidebar', 'collectionItem', 'markdown']) {
  context.avatarKind = type
  assert.throws(() => runInContext('nora.ui.node(avatarKind, {avatarBase64:"AA=="})', context), /bounded standard Base64/)
}
const legacyLabel = runInContext('nora.ui.menu({text:"Account"})', context)
assert.equal(legacyLabel.avatarBase64, undefined)
assert.equal(Object.hasOwn(legacyLabel, 'avatarBase64'), false)
assert.equal(runInContext('nora.ui.menu({text:"Account", avatarBase64:null})', context).avatarBase64, null)
assert.equal(requests.length, avatarRequestCount, 'Inline avatars never trigger transport or image requests')
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

const invalidationRequests: { method: string; params: unknown }[] = []
let invalidationResponse: () => boolean | Promise<boolean> = () => true
const invalidationContext = createContext({ transport: (method: string, params: unknown) => {
  invalidationRequests.push({ method, params })
  return invalidationResponse()
} })
const invalidationController = runInContext(`(${source})(transport)`, invalidationContext)
runInContext('nora.registerPanel("com.example.background", event => nora.ui.text(JSON.stringify(event)))', invalidationContext)
runInContext('nora.registerCommand("com.example.command", () => null)', invalidationContext)
assert.equal(runInContext('Object.isFrozen(nora.panels)', invalidationContext), true)
assert.deepEqual(JSON.parse(runInContext('JSON.stringify(Object.keys(nora.panels))', invalidationContext)), ['invalidate'])
for (const panelID of [undefined, null, '', 42, true, [], {}, 'com.example.unregistered', 'com.example.command', 'x'.repeat(256)]) {
  invalidationContext.invalidPanelID = panelID
  let result: Promise<boolean> | undefined
  assert.doesNotThrow(() => { result = runInContext('nora.panels.invalidate(invalidPanelID)', invalidationContext) })
  assert.equal(typeof result?.then, 'function', 'Invalidation validation rejects through its returned promise')
  await assert.rejects(result!, /registered panel identifier/)
}
assert.equal(invalidationRequests.length, 0, 'Unknown panels, command identifiers, and invalid input never reach the host')
const backgroundRefresh = runInContext('nora.panels.invalidate("com.example.background", {actionCapability:"ignored"})', invalidationContext)
assert.equal(invalidationRequests.length, 0, 'Host calls are scheduled asynchronously even for synchronous transports')
assert.equal(await backgroundRefresh, true)
assert.deepEqual(JSON.parse(JSON.stringify(invalidationRequests)), [{ method: 'panels.invalidate', params: { panelID: 'com.example.background' } }])
invalidationResponse = () => false
assert.equal(await runInContext('nora.panels.invalidate("com.example.background")', invalidationContext), false, 'Host refusal is preserved')
const maximumPanelID = 'x'.repeat(255)
invalidationContext.maximumPanelID = maximumPanelID
runInContext('nora.registerPanel(maximumPanelID, () => nora.ui.text("Maximum identifier"))', invalidationContext)
assert.throws(() => runInContext('nora.registerPanel("x".repeat(256), () => null)', invalidationContext), /Invalid Nora contribution/)
invalidationResponse = () => true
assert.equal(await runInContext('nora.panels.invalidate(maximumPanelID)', invalidationContext), true)
assert.deepEqual(JSON.parse(JSON.stringify(invalidationRequests.at(-1))), { method: 'panels.invalidate', params: { panelID: maximumPanelID } })
const synchronousFailure = new Error('Synchronous host failure')
invalidationResponse = () => { throw synchronousFailure }
let failedRefresh: Promise<boolean> | undefined
assert.doesNotThrow(() => { failedRefresh = runInContext('nora.panels.invalidate("com.example.background")', invalidationContext) })
await assert.rejects(failedRefresh!, error => error === synchronousFailure)
const asynchronousFailure = new Error('Asynchronous host failure')
invalidationResponse = () => Promise.reject(asynchronousFailure)
await assert.rejects(runInContext('nora.panels.invalidate("com.example.background")', invalidationContext), error => error === asynchronousFailure)
let finishRefresh: ((accepted: boolean) => void) | undefined
invalidationResponse = () => new Promise(resolve => { finishRefresh = resolve })
let refreshFinished = false
const pendingRefresh = runInContext('nora.panels.invalidate("com.example.background")', invalidationContext).then((accepted: boolean) => {
  refreshFinished = true
  return accepted
})
await Promise.resolve()
assert.equal(refreshFinished, false, 'The invalidation promise waits for the asynchronous host reply')
finishRefresh!(true)
assert.equal(await pendingRefresh, true)
const backgroundEvent = JSON.parse((await invalidationController.invokePanel('com.example.background')).text)
assert.deepEqual(backgroundEvent, { locale: 'en', values: {} }, 'Background renders do not acquire action identifiers or capabilities')
console.log('Script author API registration, transport, native views, and isolation tests passed')
