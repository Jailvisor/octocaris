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

export type WebDefaultsSetting = {
  enabled: boolean
  fields: Record<string, unknown>
}

export type SettingsSnapshot = {
  runOnStartup: boolean
  enableGpu: boolean
  pauseFullscreen: boolean
  maxFps: number
  memorySaver: boolean
  muteWallpapers: boolean
  communityIndexUrl: string
  webDefaults: WebDefaultsSetting
}

export type PlaylistSnapshot = {
  enabled: boolean
  items: string[]
  intervalSeconds: number
  random: boolean
  current: string | null
  missing: string[]
}

export type LibrarySnapshot = {
  mods: ModCard[]
  monitors: string[]
  monitorNames?: Record<string, string>
  selected: Record<string, string>
  options: Record<string, Record<string, OptionSpec>>
  errors: string[]
  /** Optional so an older host bundle still renders. */
  playlist?: PlaylistSnapshot
}

export type CommunityFile = { path: string; bytes: number }

export type CommunityInstalled = {
  version: string | null
  managed: boolean
  update_available: boolean
  folder_path: string
}

export type CommunityItem = {
  folder: string
  name: string
  author: string
  description: string
  version: string
  tags: string[]
  entry: string | null
  license: string | null
  min_app_version: string | null
  preview_url: string | null
  preview_bytes: number | null
  download_bytes: number
  sha256: string
  file_count: number
  files_truncated: boolean
  files: CommunityFile[]
  page_url: string | null
  workshop_url: string | null
  updated_at: string | null
  installed: CommunityInstalled | null
}

export type CommunityError = { code: string; detail: string }

export type CommunityJob =
  | { folder: string; phase: 'waiting' }
  | { folder: string; phase: 'downloading' | 'verifying'; received: number; total: number | null }
  | { folder: string; phase: 'installing' }
  | { folder: string; phase: 'done'; version: string }
  | { folder: string; phase: 'failed'; error: CommunityError }
  | { folder: string; phase: 'cancelled' }

export type CommunitySnapshot = {
  status: 'idle' | 'loading' | 'ready' | 'error'
  source: string
  fetched_at: number | null
  from_cache: boolean
  error: CommunityError | null
  warnings: string[]
  job: CommunityJob | null
  items: CommunityItem[]
}

export type UiResponse =
  | { type: 'pong'; id: number; version: string }
  | { type: 'wallpaper-state'; active: boolean }
  | { type: 'desktop-icons'; visible: boolean }
  | { type: 'wallpaper-visibility'; visible: boolean }
  | { type: 'settings'; data: SettingsSnapshot }
  | { type: 'library'; data: LibrarySnapshot }
  | { type: 'community'; data: CommunitySnapshot }
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
