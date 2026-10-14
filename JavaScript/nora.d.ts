export type NoraPluginViewKind = 'column' | 'row' | 'text' | 'button' | 'textField' | 'textEditor' | 'secureField' | 'picker' | 'list' | 'divider' | 'spacer' | 'badge' | 'progress'
  | 'workspace' | 'sidebar' | 'toolbar' | 'navigationItem' | 'collection' | 'collectionItem' | 'menu' | 'emptyState' | 'markdown'
export type NoraPluginTextStyle = 'title' | 'headline' | 'body' | 'caption' | 'monospaced'
export type NoraPluginTone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger' | 'muted'
export type NoraPluginAppearance = 'plain' | 'primary' | 'secondary' | 'icon'
export type NoraPluginPermission = 'hostInfo' | 'notifications' | 'network' | 'credentials' | 'openExternal' | 'chat'

/** New conversation content for review; Nora does not send it or expose existing chats to the plugin. */
export interface NoraPluginChatDraft {
  /** Nonempty plain text, at most 256 UTF-8 bytes. */
  title: string
  /** Nonempty text, at most 4 KiB UTF-8. */
  prompt: string
  /** Plugin-supplied source material, at most 128 KiB UTF-8. */
  context: string
  /** Optional HTTPS source, without URL credentials, at most 8192 UTF-8 bytes. */
  sourceURL?: string
}
export type NoraPluginChatDraftRequest = NoraPluginChatDraft & { actionCapability: string }

export interface NoraPluginView {
  type: NoraPluginViewKind
  id?: string
  text?: string
  placeholder?: string
  /** Item actions select a row; collection.action opens it with Space/Enter; workspace.action closes its inspector. textEditor never submits on Return; use a separate button action. */
  action?: string
  /** sidebar/collection selection is a child id. Native events write values[parent.id] = selectedChild.id. textEditor preserves multiline plain text, at most 64 KiB UTF-8, bound through values[id]; other values are limited to 4 KiB. */
  value?: string
  /** Sidebar-only action invoked when the list reaches its bottom. Pair with loadMoreToken; both strings must be nonempty and at most 256 UTF-8 bytes. Omit both when no more pages remain. */
  loadMoreAction?: string
  /** Sidebar-only opaque pagination revision. Change after each page so the host can request the next page once. */
  loadMoreToken?: string
  options?: { value: string; text: string }[]
  items?: NoraPluginView[]
  /** workspace: [optional sidebar, main, optional inspector]; navigationItem/collectionItem: metadata such as status badges; menu: secondary controls. */
  children?: NoraPluginView[]
  disabled?: boolean
  subtitle?: string
  /** An SF Symbol name rendered by the native host. */
  symbol?: string
  textStyle?: NoraPluginTextStyle
  tone?: NoraPluginTone
  appearance?: NoraPluginAppearance
  /** Inline standard Base64 data for a native menu, button, or navigationItem avatar; at most 128 KiB characters / 96 KiB decoded. No URL or data: prefix. */
  avatarBase64?: string
  /** Badge-only static SVG glyph, nonempty and at most 16 KiB UTF-8. The native host validates static vector markup and renders it as a template tinted by tone; symbol remains the fallback. No scripts, entities, or linked resources. */
  iconSVG?: string
}

export interface NoraPluginPanelEvent {
  action?: string
  id?: string
  value?: string
  locale?: string
  values: Record<string, string>
  /** Opaque, short-lived authorization supplied only for the current explicit panel action. */
  actionCapability?: string
}

export type NoraPluginViewProperties = Omit<NoraPluginView, 'type'>
export type NoraPluginUI = Record<NoraPluginViewKind, (properties?: NoraPluginViewProperties) => NoraPluginView> & {
  text(text: string): NoraPluginView
  node(type: NoraPluginViewKind, properties?: NoraPluginViewProperties, children?: NoraPluginView[]): NoraPluginView
}

export interface NoraPluginAPI {
  readonly locale: string
  registerPanel(id: string, handler: (event: NoraPluginPanelEvent) => NoraPluginView | Promise<NoraPluginView>): void
  registerCommand(id: string, handler: (arguments_: Record<string, unknown>) => unknown | Promise<unknown>): void
  ui: NoraPluginUI
  readonly chat: { openDraft(draft: NoraPluginChatDraft, event: NoraPluginPanelEvent): Promise<boolean> }
  request(method: 'network.request', parameters: { url: string; method?: string; headers?: Record<string, string>; body?: string }): Promise<{ status: number; headers: Record<string, string>; bodyText: string }>
  request(method: 'credentials.get', parameters: { key: string }): Promise<string | null>
  request(method: 'credentials.set', parameters: { key: string; value: string }): Promise<boolean>
  request(method: 'credentials.delete', parameters: { key: string }): Promise<boolean>
  request(method: 'browser.open', parameters: { url: string }): Promise<boolean>
  request(method: 'timers.delay', parameters: { milliseconds: number }): Promise<boolean>
  request(method: 'chat.openDraft', parameters: NoraPluginChatDraftRequest): Promise<boolean>
  request(method: string, parameters?: Record<string, unknown>): Promise<unknown>
}

export default function createNoraPluginAPI(
  request: (method: string, parameters: Record<string, unknown>) => unknown | Promise<unknown>, locale?: string
): {
  invokePanel(id: string, event?: Partial<NoraPluginPanelEvent>): Promise<NoraPluginView>
  invokeCommand(id: string, arguments_?: Record<string, unknown>): Promise<unknown>
}

declare global {
  const nora: NoraPluginAPI
}
