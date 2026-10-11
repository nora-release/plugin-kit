export type NoraPluginViewKind = 'column' | 'row' | 'text' | 'button' | 'textField' | 'secureField' | 'picker' | 'list' | 'divider' | 'spacer' | 'badge' | 'progress'

export interface NoraPluginView {
  type: NoraPluginViewKind
  id?: string
  text?: string
  placeholder?: string
  action?: string
  value?: string
  options?: { value: string; text: string }[]
  items?: NoraPluginView[]
  children?: NoraPluginView[]
  disabled?: boolean
}

export interface NoraPluginPanelEvent {
  action?: string
  id?: string
  value?: string
  locale?: string
  values: Record<string, string>
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
  request(method: 'network.request', parameters: { url: string; method?: string; headers?: Record<string, string>; body?: string }): Promise<{ status: number; headers: Record<string, string>; bodyText: string }>
  request(method: 'credentials.get', parameters: { key: string }): Promise<string | null>
  request(method: 'credentials.set', parameters: { key: string; value: string }): Promise<boolean>
  request(method: 'credentials.delete', parameters: { key: string }): Promise<boolean>
  request(method: 'browser.open', parameters: { url: string }): Promise<boolean>
  request(method: 'timers.delay', parameters: { milliseconds: number }): Promise<boolean>
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
