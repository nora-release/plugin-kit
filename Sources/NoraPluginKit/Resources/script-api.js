export default function createNoraPluginAPI(request, locale = 'en') {
  if (typeof request !== 'function') throw new Error('Nora host transport is unavailable.')
  const panels = new Map()
  const commands = new Map()
  const kinds = ['column', 'row', 'text', 'button', 'textField', 'textEditor', 'secureField', 'picker', 'list', 'divider', 'spacer', 'badge', 'progress',
    'workspace', 'sidebar', 'toolbar', 'detail', 'tabBar', 'splitView', 'itemRow', 'navigationItem', 'collection', 'collectionItem', 'menu', 'emptyState', 'markdown', 'diff']
  const textStyles = ['title', 'headline', 'body', 'caption', 'monospaced']
  const tones = ['neutral', 'accent', 'success', 'warning', 'danger', 'muted']
  const appearances = ['plain', 'primary', 'secondary', 'icon']
  const base64Alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'
  const validBoundedString = (value, maximumBytes) => {
    if (typeof value !== 'string' || !value.length) return false
    let bytes = 0
    for (const character of value) {
      const codePoint = character.codePointAt(0)
      if (codePoint >= 0xd800 && codePoint <= 0xdfff) return false
      bytes += codePoint <= 0x7f ? 1 : codePoint <= 0x7ff ? 2 : codePoint <= 0xffff ? 3 : 4
      if (bytes > maximumBytes) return false
    }
    return true
  }
  const validAvatar = value => {
    if (typeof value !== 'string' || !value.length || value.length > 128 * 1024
      || !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(value)) return false
    const padding = value.endsWith('==') ? 2 : value.endsWith('=') ? 1 : 0
    if (value.length / 4 * 3 - padding > 96 * 1024) return false
    if (padding === 2 && (base64Alphabet.indexOf(value[value.length - 3]) & 15) !== 0) return false
    if (padding === 1 && (base64Alphabet.indexOf(value[value.length - 2]) & 3) !== 0) return false
    return true
  }
  const validateOptionalRowAction = value => {
    if (value != null && (!validBoundedString(value, 256) || !value.trim())) {
      throw new Error('Nora native row action must be omitted for read-only content or a nonempty string of at most 256 UTF-8 bytes.')
    }
  }
  const hasOnlyStaticMetadata = children => {
    const pending = [...children]
    let count = 0
    while (pending.length) {
      const child = pending.pop()
      count += 1
      if (!child || typeof child !== 'object' || count > 1000
        || !['row', 'column', 'text', 'badge', 'progress', 'spacer', 'divider'].includes(child.type)
        || child.action != null || (child.items != null && (!Array.isArray(child.items) || child.items.length))
        || (child.options != null && (!Array.isArray(child.options) || child.options.length))) return false
      if (child.children != null) {
        if (!Array.isArray(child.children)) return false
        pending.push(...child.children)
      }
    }
    return true
  }
  const validateDetailShape = view => {
    if (!['detail', 'tabBar', 'splitView', 'itemRow'].includes(view.type)) return
    const children = view.children ?? []
    if (!Array.isArray(children) || !children.every(child => child && typeof child === 'object' && !Array.isArray(child) && kinds.includes(child.type))
      || (view.items != null && (!Array.isArray(view.items) || view.items.length))) {
      throw new Error('Nora detail views require child arrays and do not accept items.')
    }
    if (view.type === 'detail' && (children.length < 4 || children[0]?.type !== 'toolbar'
      || children[1]?.type !== 'column' || children[2]?.type !== 'tabBar')) {
      throw new Error('Nora detail requires a toolbar, heading column, tab bar, and body content in that order.')
    }
    if (view.type === 'tabBar') {
      if (!children.length || !children.every(child => child?.type === 'button' && (child.value == null || child.value === 'selected'))
        || children.filter(child => child.value === 'selected').length > 1) {
        throw new Error('Nora tab bar requires button children with at most one selected value.')
      }
      for (const child of children) validateOptionalRowAction(child.action)
    }
    if (view.type === 'splitView') {
      if (children.length !== 2 || (view.value != null && !['list', 'detail'].includes(view.value))) {
        throw new Error('Nora split view requires two child panes and an optional list or detail value.')
      }
      validateOptionalRowAction(view.action)
    }
    if (view.type === 'itemRow') {
      if ((view.value != null && view.value !== 'selected') || !hasOnlyStaticMetadata(children)) {
        throw new Error('Nora item row requires static metadata children and an optional selected value.')
      }
      validateOptionalRowAction(view.action)
    }
  }
  const validateDiffShape = view => {
    if (view.type !== 'diff') return
    if (!validBoundedString(view.text, 64 * 1024)
      || !validBoundedString(view.subtitle, 4096) || !view.subtitle.trim()
      || /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/u.test(view.subtitle)
      || (view.value != null && !['unified', 'split'].includes(view.value))
      || ['children', 'items', 'options'].some(key => view[key] != null && (!Array.isArray(view[key]) || view[key].length))
      || ['action', 'placeholder', 'symbol', 'textStyle', 'tone', 'appearance', 'avatarBase64', 'iconSVG',
        'loadMoreAction', 'loadMoreToken'].some(key => view[key] != null)) {
      throw new Error('Nora diff requires a nonempty patch of at most 64 KiB UTF-8, a single-line filename hint of at most 4 KiB UTF-8, an optional unified/split layout, and no interactive or styled content.')
    }
  }
  const node = (type, properties = {}, children) => {
    if (!kinds.includes(type)) throw new Error('Unsupported Nora view type.')
    if (typeof properties === 'string') properties = { text: properties }
    if (properties === null || typeof properties !== 'object' || Array.isArray(properties)) throw new Error('Nora view properties must be an object.')
    for (const [key, choices] of [['textStyle', textStyles], ['tone', tones], ['appearance', appearances]]) {
      if (properties[key] != null && !choices.includes(properties[key])) throw new Error(`Unsupported Nora view ${key}.`)
    }
    if (type === 'textEditor' && properties.value != null && properties.value !== '' && !validBoundedString(properties.value, 64 * 1024)) {
      throw new Error('Nora text editor value must be text of at most 64 KiB UTF-8.')
    }
    if (properties.avatarBase64 != null && (!['menu', 'button', 'navigationItem'].includes(type) || !validAvatar(properties.avatarBase64))) {
      throw new Error('Nora view avatar must contain bounded standard Base64 data on a native label.')
    }
    if (properties.iconSVG != null && (type !== 'badge' || !validBoundedString(properties.iconSVG, 16 * 1024))) {
      throw new Error('Nora view icon SVG must be a nonempty string of at most 16 KiB UTF-8 on a badge.')
    }
    if ((properties.loadMoreAction != null || properties.loadMoreToken != null)
      && (type !== 'sidebar' || !validBoundedString(properties.loadMoreAction, 256) || !validBoundedString(properties.loadMoreToken, 256))) {
      throw new Error('Nora sidebar pagination requires paired nonempty action and token strings of at most 256 UTF-8 bytes on a sidebar.')
    }
    const view = { ...properties, ...(children === undefined ? {} : { children }), type }
    validateDetailShape(view)
    validateDiffShape(view)
    return view
  }
  const ui = Object.fromEntries(kinds.map(type => [type, properties => node(type, properties)]))
  ui.node = node
  const validContributionID = id => typeof id === 'string' && id.length > 0 && id.length <= 255
  const register = (collection, id, handler) => {
    if (!validContributionID(id) || typeof handler !== 'function') throw new Error('Invalid Nora contribution.')
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
    panels: Object.freeze({
      invalidate: panelID => Promise.resolve().then(() => {
        if (!validContributionID(panelID) || !panels.has(panelID)) {
          throw new Error('Invalidating a Nora panel requires a registered panel identifier of at most 255 characters.')
        }
        return request('panels.invalidate', { panelID })
      }),
    }),
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
