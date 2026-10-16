export type NoraPluginViewKind = 'column' | 'row' | 'text' | 'button' | 'textField' | 'textEditor' | 'secureField' | 'picker' | 'list' | 'divider' | 'spacer' | 'badge' | 'progress'
  | 'workspace' | 'sidebar' | 'toolbar' | 'detail' | 'timelineItem' | 'tabBar' | 'splitView' | 'itemRow' | 'navigationItem' | 'collection' | 'collectionItem' | 'menu' | 'emptyState' | 'markdown' | 'diff'
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
  /** diff: nonempty raw unified patch, at most 64 KiB UTF-8. Rendered read-only by Nora's native diff viewer. */
  text?: string
  placeholder?: string
  /** Item actions select a row; collection.action opens it with Space/Enter; workspace.action closes its inspector. splitView.action returns to its list at narrow widths; text labels that button. Omit itemRow.action for a read-only row; provided row/back actions must be nonempty and at most 256 UTF-8 bytes. textEditor never submits on Return; use a separate button action. */
  action?: string
  /** sidebar/collection selection is a child id. Native events write values[parent.id] = selectedChild.id. A tabBar button or itemRow uses "selected" for selection. splitView uses "detail" to show the detail pane at narrow widths, otherwise its list (omit or use "list"). diff accepts "unified" (default) or "split" layout. textEditor preserves multiline plain text, at most 64 KiB UTF-8, bound through values[id]; other values are limited to 4 KiB. */
  value?: string
  /** Sidebar-only action invoked when the list reaches its bottom. Pair with loadMoreToken; both strings must be nonempty and at most 256 UTF-8 bytes. Omit both when no more pages remain. */
  loadMoreAction?: string
  /** Sidebar-only opaque pagination revision. Change after each page so the host can request the next page once. */
  loadMoreToken?: string
  options?: { value: string; text: string }[]
  /** detail: optional single column for a separate summary aside (SDK 0.14+). Its tree accepts column/row/text/button/picker/menu/divider/spacer/badge/progress/emptyState/markdown/itemRow, and no nested items. The host controls the aside width, scrolling, and compact placement. Existing detail children keep their original order. Other native detail layouts (tabBar/splitView/itemRow/timelineItem) never accept items. */
  items?: NoraPluginView[]
  /** workspace: [optional sidebar, main, optional inspector]; detail: [toolbar, heading/summary column, tabBar, ...body] with a fixed heading and scrolling body. A sole splitView body fills remaining height. tabBar: button children (subtitle may show a count), at most one selected; id/action/disabled remain on each button. splitView: exactly [list, detail], a 248-point list beside flexible detail, independently scrolling. itemRow: static trailing text/badge/progress/spacer/divider, optionally grouped in row/column, without actions or input controls; tone colors only the row's leading SVG or symbol. navigationItem/collectionItem: metadata such as status badges; menu: secondary controls. */
  children?: NoraPluginView[]
  disabled?: boolean
  /** diff: required nonempty filename/language hint, at most 4 KiB UTF-8 without control, format, or newline characters. Display metadata only; never a request to read a local file. diff rejects nonempty children/items/options and action, placeholder, symbol, textStyle, tone, appearance, avatars, icons, or pagination. */
  subtitle?: string
  /** An SF Symbol name rendered by the native host. */
  symbol?: string
  textStyle?: NoraPluginTextStyle
  tone?: NoraPluginTone
  appearance?: NoraPluginAppearance
  /** Inline standard Base64 data for a native menu, button, or navigationItem avatar; at most 128 KiB characters / 96 KiB decoded. No URL or data: prefix. */
  avatarBase64?: string
  /** Static SVG glyph on badge or itemRow, nonempty and at most 16 KiB UTF-8. The native host validates static vector markup and renders it as a template tinted by tone; symbol remains the fallback. No scripts, entities, or linked resources. Gate itemRow SVGs with nora.features?.itemRowIcon === true on SDK 0.15+; older hosts retain the symbol fallback. */
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
/** Native read-only patch content. Filename hints never request local file access. */
export interface NoraPluginDiffProperties {
  id?: string
  text: string
  subtitle: string
  value?: 'unified' | 'split'
  disabled?: boolean
}
/**
 * Native discussion entry. Children are [author button, metadata row, optional body column].
 * The host places the existing author button's avatar in a leading gutter and its name in the header;
 * the author action and disabled state are retained. Metadata and body retain their own controls.
 * Omitted value means "comment"; "event" uses a compact activity layout. No container action or items.
 * Older hosts do not expose ui.timelineItem; feature-detect it before returning this kind.
 */
export interface NoraPluginTimelineItemProperties {
  id?: string
  value?: 'comment' | 'event'
  disabled?: boolean
  children: [NoraPluginView, NoraPluginView] | [NoraPluginView, NoraPluginView, NoraPluginView]
}
export type NoraPluginUI = Record<Exclude<NoraPluginViewKind, 'diff' | 'timelineItem'>, (properties?: NoraPluginViewProperties) => NoraPluginView> & {
  diff(properties: NoraPluginDiffProperties): NoraPluginView
  timelineItem(properties: NoraPluginTimelineItemProperties): NoraPluginView
  text(text: string): NoraPluginView
  node(type: NoraPluginViewKind, properties?: NoraPluginViewProperties, children?: NoraPluginView[]): NoraPluginView
}

export interface NoraPluginAPI {
  readonly locale: string
  /** Optional layout capabilities supplied by this SDK's matching native host. Older hosts may omit features. */
  readonly features: Readonly<{ detailAside: boolean; itemRowIcon: boolean }>
  registerPanel(id: string, handler: (event: NoraPluginPanelEvent) => NoraPluginView | Promise<NoraPluginView>): void
  registerCommand(id: string, handler: (arguments_: Record<string, unknown>) => unknown | Promise<unknown>): void
  ui: NoraPluginUI
  readonly panels: {
    /**
     * Request a fresh render after background data changes, without authorizing a user action.
     * panelID must already be registered by this SDK instance and contain 1–255 UTF-16 code units,
     * matching registerPanel; the host also checks ownership. Resolves to the host's refresh
     * acceptance, not render completion. Invalid identifiers and transport failures reject
     * asynchronously. Older hosts may reject this optional request.
     */
    invalidate(panelID: string): Promise<boolean>
  }
  readonly chat: { openDraft(draft: NoraPluginChatDraft, event: NoraPluginPanelEvent): Promise<boolean> }
  request(method: 'network.request', parameters: { url: string; method?: string; headers?: Record<string, string>; body?: string }): Promise<{ status: number; headers: Record<string, string>; bodyText: string }>
  request(method: 'credentials.get', parameters: { key: string }): Promise<string | null>
  request(method: 'credentials.set', parameters: { key: string; value: string }): Promise<boolean>
  request(method: 'credentials.delete', parameters: { key: string }): Promise<boolean>
  request(method: 'browser.open', parameters: { url: string }): Promise<boolean>
  request(method: 'timers.delay', parameters: { milliseconds: number }): Promise<boolean>
  request(method: 'panels.invalidate', parameters: { panelID: string }): Promise<boolean>
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
