export type OptionSpec = {
  type?: string
  label?: string
  value?: unknown
  min?: number
  max?: number
  step?: number
  options?: string[]
  [key: string]: unknown
}

export type ModCard = {
  folder_name: string
  folder_path: string
  name: string
  author: string
  description: string
  image_url: string | null
  entry_url: string | null
  authorsite: string | null
  source: string | null
  version: string | null
  keywords: string[]
  options: Record<string, OptionSpec>
}

export type SettingsSnapshot = {
  runOnStartup: boolean
  enableGpu: boolean
  pauseFullscreen: boolean
  maxFps: number
  memorySaver: boolean
  liteMode: boolean
}

export type LibrarySnapshot = {
  mods: ModCard[]
  monitors: string[]
  selected: Record<string, string>
  options: Record<string, Record<string, OptionSpec>>
  errors: string[]
}

export type UiResponse =
  | { type: 'pong'; id: number; version: string }
  | { type: 'wallpaper-state'; active: boolean }
  | { type: 'desktop-icons'; visible: boolean }
  | { type: 'settings'; data: SettingsSnapshot }
  | { type: 'library'; data: LibrarySnapshot }
  | { type: 'notice'; message: string }
  | { type: 'error'; message: string }

type WebViewBridge = {
  postMessage(message: unknown): void
  postMessageWithAdditionalObjects?(message: unknown, objects: unknown[]): void
  addEventListener(type: 'message', listener: (event: MessageEvent) => void): void
  removeEventListener(type: 'message', listener: (event: MessageEvent) => void): void
}

declare global {
  interface Window {
    chrome?: { webview?: WebViewBridge }
  }
}

function bridge(): WebViewBridge | undefined {
  return window.chrome?.webview
}

export function isHosted(): boolean {
  return bridge() !== undefined
}

export function send(message: unknown): void {
  const webview = bridge()
  if (webview) {
    webview.postMessage(message)
  } else {
    console.info('[octos mock] ->', message)
  }
}

export function subscribe(listener: (message: UiResponse) => void): () => void {
  const webview = bridge()
  if (!webview) {
    return () => {}
  }

  const handler = (event: MessageEvent) => listener(event.data as UiResponse)
  webview.addEventListener('message', handler)
  return () => webview.removeEventListener('message', handler)
}
