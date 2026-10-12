export default function createNoraPluginAPI(request, locale = 'en') {
  if (typeof request !== 'function') throw new Error('Nora host transport is unavailable.')
  const panels = new Map()
  const commands = new Map()
  const kinds = ['column', 'row', 'text', 'button', 'textField', 'secureField', 'picker', 'list', 'divider', 'spacer', 'badge', 'progress',
    'workspace', 'sidebar', 'toolbar', 'navigationItem', 'collection', 'collectionItem', 'menu', 'emptyState', 'markdown']
  const textStyles = ['title', 'headline', 'body', 'caption', 'monospaced']
  const tones = ['neutral', 'accent', 'success', 'warning', 'danger', 'muted']
  const appearances = ['plain', 'primary', 'secondary', 'icon']
  const node = (type, properties = {}, children) => {
    if (!kinds.includes(type)) throw new Error('Unsupported Nora view type.')
    if (typeof properties === 'string') properties = { text: properties }
    if (properties === null || typeof properties !== 'object' || Array.isArray(properties)) throw new Error('Nora view properties must be an object.')
    for (const [key, choices] of [['textStyle', textStyles], ['tone', tones], ['appearance', appearances]]) {
      if (properties[key] != null && !choices.includes(properties[key])) throw new Error(`Unsupported Nora view ${key}.`)
    }
    return { ...properties, ...(children === undefined ? {} : { children }), type }
  }
  const ui = Object.fromEntries(kinds.map(type => [type, properties => node(type, properties)]))
  ui.node = node
  const register = (collection, id, handler) => {
    if (typeof id !== 'string' || id.length === 0 || id.length > 255 || typeof handler !== 'function') throw new Error('Invalid Nora contribution.')
    if (panels.has(id) || commands.has(id)) throw new Error('Duplicate Nora contribution.')
    collection.set(id, handler)
  }
  const invoke = async (collection, id, input) => {
    const handler = collection.get(id)
    if (!handler) throw new Error('This Nora contribution is not registered.')
    return await handler(input)
  }
  const api = {
    locale: String(locale),
    ui: Object.freeze(ui),
    chat: Object.freeze({
      openDraft: (draft, event) => Promise.resolve().then(() => {
        if (!event || typeof event.action !== 'string' || !event.action.trim()
          || typeof event.actionCapability !== 'string' || !event.actionCapability) {
          throw new Error('Opening a Nora draft requires an explicit panel action.')
        }
        return request('chat.openDraft', { ...draft, actionCapability: event.actionCapability })
      }),
    }),
    registerPanel: (id, handler) => register(panels, id, handler),
    registerCommand: (id, handler) => register(commands, id, handler),
    request: (method, parameters = {}) => Promise.resolve().then(() => request(method, parameters)),
  }
  Object.defineProperty(globalThis, 'nora', { value: Object.freeze(api), writable: false, configurable: false })
  return Object.freeze({
    invokePanel: (id, event = {}) => invoke(panels, id, { ...event, locale: String(locale), values: event.values ?? {} }),
    invokeCommand: (id, arguments_ = {}) => invoke(commands, id, arguments_),
  })
}
