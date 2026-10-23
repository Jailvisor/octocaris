import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'
import {
  isHosted,
  send,
  subscribe,
  type LibrarySnapshot,
  type ModCard,
  type SettingsSnapshot,
  type OptionSpec,
} from './bridge'
import { setLanguage, type AppLanguage } from './i18n'

type Tab = 'library' | 'settings'
type HostState = 'preview' | 'waiting' | 'connected' | 'error'
type NoticeTone = 'error' | 'success' | 'info'
type Notice = { message: string; tone: NoticeTone }

function LibraryIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <rect x="3" y="4" width="7" height="16" rx="2" />
      <rect x="14" y="4" width="7" height="9" rx="2" />
      <rect x="14" y="16" width="7" height="4" rx="2" />
    </svg>
  )
}

function SettingsIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="12" cy="12" r="3.2" />
      <path d="M12 2.8v3M12 18.2v3M2.8 12h3M18.2 12h3M5.5 5.5l2.1 2.1M16.4 16.4l2.1 2.1M18.5 5.5l-2.1 2.1M7.6 16.4l-2.1 2.1" />
    </svg>
  )
}

function DownloadIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 3v11m0 0 4-4m-4 4-4-4M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2" />
    </svg>
  )
}

function FolderPlusIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z" />
      <path d="M12 11v5m-2.5-2.5h5" />
    </svg>
  )
}

function ImageIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <circle cx="8.5" cy="9.5" r="1.6" />
      <path d="m3 17 5-5 4 4 3-3 6 6" />
    </svg>
  )
}

function RefreshIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M20 11a8 8 0 1 0-2.3 6.3M20 5v6h-6" />
    </svg>
  )
}

function CheckIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="m5 12.5 4.2 4.2L19 7" />
    </svg>
  )
}

function WarningIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 3.5 21 20H3L12 3.5Z" />
      <path d="M12 9.5v4.5M12 17.2v.1" />
    </svg>
  )
}

function InfoIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="12" cy="12" r="9" />
      <path d="M12 10.5V17M12 7.4v.1" />
    </svg>
  )
}

function StatusCard({
  state,
  version,
  wallpaperActive,
}: {
  state: HostState
  version: string | null
  wallpaperActive: boolean
}) {
  const { t } = useTranslation()
  const labels: Record<HostState, string> = {
    preview: t('bridge.preview'),
    waiting: t('bridge.waiting'),
    connected: t('bridge.connected'),
    error: t('bridge.error'),
  }

  return (
    <div className="panel status-panel">
      <div className="panel-heading">
        <div>
          <p className="eyebrow">{t('bridge.eyebrow')}</p>
          <h3>{labels[state]}</h3>
        </div>
        <span className={`status-dot ${state}`} />
      </div>
      <dl className="meta">
        <div>
          <dt>{t('bridge.protocol')}</dt>
          <dd>0.1</dd>
        </div>
        <div>
          <dt>{t('bridge.hostBuild')}</dt>
          <dd>{version ?? '—'}</dd>
        </div>
        <div>
          <dt>{t('bridge.transport')}</dt>
          <dd>{isHosted() ? t('bridge.webview') : t('bridge.mock')}</dd>
        </div>
        <div>
          <dt>{t('bridge.wallpaper')}</dt>
          <dd>{wallpaperActive ? t('bridge.gradientActive') : t('bridge.cleared')}</dd>
        </div>
      </dl>
    </div>
  )
}

function EmptyLibrary({
  status,
  active,
  onShow,
  onClear,
  onInstall,
  onImport,
  onAddImage,
}: {
  status: HostState
  active: boolean
  onShow: () => void
  onClear: () => void
  onInstall: () => void
  onImport: () => void
  onAddImage: () => void
}) {
  const { t } = useTranslation()

  return (
    <div className="panel empty-state">
      <div className="empty-art" aria-hidden="true">
        <span />
        <span />
        <span />
      </div>
      <h3>{t('library.emptyTitle')}</h3>
      <p>{t('library.emptyBody')}</p>
      <div className="empty-actions">
        <button className="button primary" onClick={onAddImage}>
          <ImageIcon />
          {t('library.addImage')}
        </button>
        <button className="button" onClick={onInstall}>
          <DownloadIcon />
          {t('library.install')}
        </button>
        <button className="button" onClick={onImport}>
          <FolderPlusIcon />
          {t('library.importFolder')}
        </button>
      </div>
      <div className="empty-secondary">
        <button className="button ghost" onClick={onShow}>
          {t('common.showGradient')}
        </button>
        <button className="button ghost" onClick={onClear} disabled={!active}>
          {t('common.clearWallpaper')}
        </button>
      </div>
      <p className="tiny">
        {t('library.bridgeStatus')}: <strong>{t(`bridge.${status}`)}</strong>
      </p>
    </div>
  )
}

function prettyMonitor(id: string) {
  return id.replace(/^\\\\.\\/, '') || id
}

function OptionControl({
  id,
  spec,
  onChange,
}: {
  id: string
  spec: OptionSpec
  onChange: (id: string, value: unknown) => void
}) {
  const type = String(spec.type ?? 'checkbox')
  const label = typeof spec.label === 'string' && spec.label ? spec.label : id
  const value = spec.value
  const selectOptions = Array.isArray(spec.options)
    ? spec.options.filter((option): option is string => typeof option === 'string')
    : []

  if (type === 'description') {
    return <p className="option-description">{label}</p>
  }

  if (type === 'checkbox') {
    return (
      <label className="option-row">
        <span>{label}</span>
        <input
          type="checkbox"
          checked={value === true}
          onChange={(event) => onChange(id, event.target.checked)}
        />
      </label>
    )
  }

  if (type === 'range' || type === 'number') {
    const numeric = typeof value === 'number' ? value : Number(value ?? 0)
    return (
      <label className="option-row">
        <span>
          {label}
          <em>{String(numeric)}</em>
        </span>
        <input
          type={type}
          value={Number.isFinite(numeric) ? numeric : 0}
          min={typeof spec.min === 'number' ? spec.min : undefined}
          max={typeof spec.max === 'number' ? spec.max : undefined}
          step={typeof spec.step === 'number' ? spec.step : undefined}
          onChange={(event) => onChange(id, Number(event.target.value))}
        />
      </label>
    )
  }

  if (type === 'select' || type === 'dropdown') {
    return (
      <label className="option-row">
        <span>{label}</span>
        <select value={String(value ?? '')} onChange={(event) => onChange(id, event.target.value)}>
          {selectOptions.map((option) => (
            <option key={option} value={option}>
              {option}
            </option>
          ))}
        </select>
      </label>
    )
  }

  if (type === 'color-picker') {
    return (
      <label className="option-row">
        <span>{label}</span>
        <input
          type="color"
          value={String(value ?? '#ffffff')}
          onChange={(event) => onChange(id, event.target.value)}
        />
      </label>
    )
  }

  return (
    <label className="option-row">
      <span>{label}</span>
      <input
        type="text"
        value={typeof value === 'string' || typeof value === 'number' ? String(value) : ''}
        placeholder={type === 'file' ? 'file path' : undefined}
        onChange={(event) => onChange(id, event.target.value)}
      />
    </label>
  )
}

function ModDetail({
  card,
  options,
  monitors,
  selectedMonitor,
  selected,
  mods,
  onMonitorChange,
  onApply,
  onRemove,
  onOptionChange,
}: {
  card: ModCard
  options: Record<string, OptionSpec>
  monitors: string[]
  selectedMonitor: string
  selected: Record<string, string>
  mods: ModCard[]
  onMonitorChange: (monitor: string) => void
  onApply: () => void
  onRemove: () => void
  onOptionChange: (id: string, value: unknown) => void
}) {
  const { t } = useTranslation()
  const entries = Object.entries(options)
  const inUse = Object.values(selected).includes(card.folder_path)

  return (
    <div className="panel mod-detail">
      <div className="detail-heading">
        <div>
          <p className="eyebrow">{t('library.detail')}</p>
          <h2>{card.name}</h2>
        </div>
        {inUse && (
          <span className="pill success">
            <span className="status-dot connected" />
            {t('library.inUse')}
          </span>
        )}
      </div>
      <p className="muted">{card.description || t('library.noDescription')}</p>
      <p className="tiny">
        {t('library.author')}: {card.author || t('library.unknownAuthor')}
        {card.version ? ` · ${t('library.version')} ${card.version}` : ''}
      </p>
      {!card.entry_url && <p className="warn-text">{t('library.notRunnable')}</p>}

      <div className="apply-row">
        <label>
          {t('library.monitor')}
          <select
            value={selectedMonitor}
            onChange={(event) => onMonitorChange(event.target.value)}
            disabled={monitors.length === 0}
          >
            {monitors.map((monitor) => (
              <option key={monitor} value={monitor}>
                {prettyMonitor(monitor)}
              </option>
            ))}
          </select>
        </label>
        <button
          className="button primary"
          onClick={onApply}
          disabled={!card.entry_url || !selectedMonitor}
        >
          {t('library.setWallpaper')}
        </button>
        <button className="button ghost danger" onClick={onRemove}>
          {t('library.remove')}
        </button>
      </div>

      <div className="monitor-map">
        <h3 className="section-title">{t('library.monitorMap')}</h3>
        {Object.entries(selected).length === 0 ? (
          <p className="tiny">{t('library.noOptions')}</p>
        ) : (
          Object.entries(selected).map(([monitor, path]) => {
            const mod = mods.find((candidate) => candidate.folder_path === path)
            return (
              <div className="monitor-chip" key={monitor}>
                <span>{prettyMonitor(monitor)}</span>
                <strong>{mod?.name ?? path}</strong>
              </div>
            )
          })
        )}
      </div>

      <h3 className="section-title">{t('library.options')}</h3>
      {entries.length === 0 ? (
        <p className="tiny">{t('library.noOptions')}</p>
      ) : (
        <div className="option-list">
          {entries.map(([id, spec]) => (
            <OptionControl key={id} id={id} spec={spec} onChange={onOptionChange} />
          ))}
        </div>
      )}
    </div>
  )
}

function ModCardTile({
  card,
  index,
  selected,
  tabbable,
  onSelect,
}: {
  card: ModCard
  index: number
  selected: boolean
  tabbable: boolean
  onSelect: (path: string) => void
}) {
  const { t } = useTranslation()
  const [imageState, setImageState] = useState<'idle' | 'ready' | 'error'>(
    card.image_url ? 'idle' : 'error',
  )

  useEffect(() => {
    setImageState(card.image_url ? 'idle' : 'error')
  }, [card.image_url])

  return (
    <button
      type="button"
      role="option"
      aria-selected={selected}
      tabIndex={tabbable ? 0 : -1}
      data-mod-card
      data-index={index}
      className={selected ? 'mod-card selected' : 'mod-card'}
      onClick={() => onSelect(card.folder_path)}
      onFocus={() => onSelect(card.folder_path)}
    >
      <div className="mod-card-art">
        {card.image_url && imageState !== 'error' && (
          <img
            src={card.image_url}
            alt=""
            loading="lazy"
            decoding="async"
            className={imageState === 'ready' ? 'loaded' : undefined}
            onLoad={() => setImageState('ready')}
            onError={() => setImageState('error')}
          />
        )}
        {imageState !== 'ready' && (
          <span className="mod-card-initial" aria-hidden="true">
            {card.name.slice(0, 1).toUpperCase()}
          </span>
        )}
        {selected && (
          <span className="mod-card-check" aria-hidden="true">
            <CheckIcon />
          </span>
        )}
      </div>
      <div className="mod-card-body">
        <h3>{card.name}</h3>
        <p>{card.author || t('library.unknownAuthor')}</p>
      </div>
    </button>
  )
}

function ModGrid({
  mods,
  selectedPath,
  onSelect,
}: {
  mods: ModCard[]
  selectedPath: string | null
  onSelect: (path: string) => void
}) {
  const { t } = useTranslation()
  const gridRef = useRef<HTMLDivElement>(null)

  function handleKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    const keys = ['ArrowRight', 'ArrowLeft', 'ArrowDown', 'ArrowUp', 'Home', 'End']
    if (!keys.includes(event.key)) {
      return
    }
    const grid = gridRef.current
    if (!grid) {
      return
    }
    const cards = Array.from(grid.querySelectorAll<HTMLButtonElement>('[data-mod-card]'))
    const current = cards.indexOf(document.activeElement as HTMLButtonElement)
    if (current < 0) {
      return
    }
    const columns = Math.max(
      1,
      getComputedStyle(grid).gridTemplateColumns.split(' ').filter(Boolean).length,
    )
    let next = current
    if (event.key === 'ArrowRight') next = Math.min(current + 1, cards.length - 1)
    if (event.key === 'ArrowLeft') next = Math.max(current - 1, 0)
    if (event.key === 'ArrowDown') next = Math.min(current + columns, cards.length - 1)
    if (event.key === 'ArrowUp') next = Math.max(current - columns, 0)
    if (event.key === 'Home') next = 0
    if (event.key === 'End') next = cards.length - 1
    event.preventDefault()
    cards[next]?.focus()
  }

  return (
    <div
      ref={gridRef}
      className="mod-grid"
      role="listbox"
      aria-label={t('library.title')}
      title={t('library.keyboardHint')}
      onKeyDown={handleKeyDown}
    >
      {mods.map((card, index) => (
        <ModCardTile
          key={card.folder_path}
          card={card}
          index={index}
          selected={card.folder_path === selectedPath}
          tabbable={
            card.folder_path === selectedPath || (!selectedPath && index === 0)
          }
          onSelect={onSelect}
        />
      ))}
    </div>
  )
}

function SkeletonGrid() {
  return (
    <div className="mod-grid" aria-hidden="true">
      {[0, 1, 2, 3].map((index) => (
        <div className="mod-card skeleton-card" key={index}>
          <div className="mod-card-art skeleton" />
          <div className="mod-card-body">
            <span className="skeleton skeleton-line skeleton-title" />
            <span className="skeleton skeleton-line skeleton-subtitle" />
          </div>
        </div>
      ))}
    </div>
  )
}

function DetailSkeleton() {
  return (
    <div className="panel mod-detail" aria-hidden="true">
      <span className="skeleton skeleton-line skeleton-eyebrow" />
      <span className="skeleton skeleton-line skeleton-heading" />
      <span className="skeleton skeleton-line skeleton-body" />
      <span className="skeleton skeleton-line skeleton-body short" />
      <span className="skeleton skeleton-block" />
    </div>
  )
}

function SettingSwitch({
  checked,
  label,
  onChange,
}: {
  checked: boolean
  label: string
  onChange: (value: boolean) => void
}) {
  return (
    <button
      className={checked ? 'switch interactive active' : 'switch interactive'}
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
    >
      <span />
    </button>
  )
}

function SettingsView({
  desktopIcons,
  onToggleDesktopIcons,
  settings,
  onSetSetting,
}: {
  desktopIcons: boolean
  onToggleDesktopIcons: () => void
  settings: SettingsSnapshot
  onSetSetting: (key: keyof SettingsSnapshot, value: boolean | number) => void
}) {
  const { t, i18n } = useTranslation()
  const language: AppLanguage = i18n.language.toLowerCase().startsWith('zh') ? 'zh-CN' : 'en'
  const pendingRows: Array<[string, string, string]> = [
    [t('settings.allowNotifications.title'), t('settings.allowNotifications.desc'), 'M8'],
    [t('settings.memorySaver.title'), t('settings.memorySaver.desc'), 'M6'],
  ]

  return (
    <div className="panel settings-panel">
      <div className="setting-row">
        <div>
          <h3>{t('settings.showDesktopIcons.title')}</h3>
          <p>{t('settings.showDesktopIcons.desc')}</p>
        </div>
        <div className="setting-tail">
          <span className="badge">M2</span>
          <button
            className={desktopIcons ? 'switch interactive active' : 'switch interactive'}
            role="switch"
            aria-checked={desktopIcons}
            aria-label={t('settings.showDesktopIcons.title')}
            onClick={onToggleDesktopIcons}
          >
            <span />
          </button>
        </div>
      </div>
      <div className="setting-row">
        <div>
          <h3>{t('settings.language.title')}</h3>
          <p>{t('settings.language.desc')}</p>
        </div>
        <div className="setting-tail">
          <span className="badge">M4</span>
          <select
            className="language-select"
            value={language}
            aria-label={t('settings.language.title')}
            onChange={(event) => setLanguage(event.target.value as AppLanguage)}
          >
            <option value="en">{t('language.en')}</option>
            <option value="zh-CN">{t('language.zhCN')}</option>
          </select>
        </div>
      </div>
      <div className="setting-row">
        <div>
          <h3>{t('settings.pauseFullscreen.title')}</h3>
          <p>{t('settings.pauseFullscreen.desc')}</p>
        </div>
        <div className="setting-tail">
          <span className="badge">M4</span>
          <SettingSwitch
            checked={settings.pauseFullscreen}
            label={t('settings.pauseFullscreen.title')}
            onChange={(value) => onSetSetting('pauseFullscreen', value)}
          />
        </div>
      </div>
      <div className="setting-row">
        <div>
          <h3>{t('settings.startWithWindows.title')}</h3>
          <p>{t('settings.startWithWindows.desc')}</p>
        </div>
        <div className="setting-tail">
          <span className="badge">M4</span>
          <SettingSwitch
            checked={settings.runOnStartup}
            label={t('settings.startWithWindows.title')}
            onChange={(value) => onSetSetting('runOnStartup', value)}
          />
        </div>
      </div>
      <div className="setting-row">
        <div>
          <h3>{t('settings.maxFps.title')}</h3>
          <p>{t('settings.maxFps.desc')}</p>
        </div>
        <div className="setting-tail">
          <span className="badge">M5</span>
          <select
            className="language-select"
            value={settings.maxFps}
            aria-label={t('settings.maxFps.title')}
            onChange={(event) => onSetSetting('maxFps', Number(event.target.value))}
          >
            <option value={15}>15 FPS</option>
            <option value={30}>30 FPS</option>
            <option value={60}>60 FPS</option>
            <option value={0}>{t('settings.maxFps.unlimited')}</option>
          </select>
        </div>
      </div>
      <div className="setting-row">
        <div>
          <h3>{t('settings.enableGpu.title')}</h3>
          <p>{t('settings.enableGpu.desc')}</p>
        </div>
        <div className="setting-tail">
          <span className="badge">M4</span>
          <SettingSwitch
            checked={settings.enableGpu}
            label={t('settings.enableGpu.title')}
            onChange={(value) => onSetSetting('enableGpu', value)}
          />
        </div>
      </div>
      {pendingRows.map(([title, description, milestone]) => (
        <div className="setting-row" key={title}>
          <div>
            <h3>{title}</h3>
            <p>{description}</p>
          </div>
          <div className="setting-tail">
            <span className="badge">{milestone}</span>
            <span className="switch" aria-hidden="true">
              <span />
            </span>
          </div>
        </div>
      ))}
    </div>
  )
}

export default function App() {
  const { t } = useTranslation()
  const [tab, setTab] = useState<Tab>('library')
  const [hostState, setHostState] = useState<HostState>(isHosted() ? 'waiting' : 'preview')
  const [version, setVersion] = useState<string | null>(null)
  const [wallpaperActive, setWallpaperActive] = useState(true)
  const [desktopIcons, setDesktopIcons] = useState(true)
  const [settings, setSettings] = useState<SettingsSnapshot>({
    runOnStartup: false,
    enableGpu: true,
    pauseFullscreen: true,
    maxFps: 30,
  })
  const [library, setLibrary] = useState<LibrarySnapshot | null>(null)
  const [selectedModPath, setSelectedModPath] = useState<string | null>(null)
  const [selectedMonitor, setSelectedMonitor] = useState('')
  const [installing, setInstalling] = useState<'mod' | 'folder' | 'image' | null>(null)
  const [dragging, setDragging] = useState(false)
  const [notice, setNotice] = useState<Notice | null>(null)
  const installingRef = useRef<'mod' | 'folder' | 'image' | null>(null)
  const installBaselineRef = useRef(0)

  useEffect(() => {
    installingRef.current = installing
  }, [installing])

  useEffect(() => {
    function onDragOver(event: DragEvent) {
      if (!event.dataTransfer) {
        return
      }
      event.preventDefault()
      event.dataTransfer.dropEffect = 'copy'
      if (event.dataTransfer.types.includes('Files')) {
        setDragging(true)
      }
    }
    function onDragLeave(event: DragEvent) {
      if (event.relatedTarget === null) {
        setDragging(false)
      }
    }
    function onDrop(event: DragEvent) {
      event.preventDefault()
      setDragging(false)
      const files = Array.from(event.dataTransfer?.files ?? [])
      if (files.length === 0) {
        return
      }
      const webview = window.chrome?.webview
      if (webview?.postMessageWithAdditionalObjects) {
        webview.postMessageWithAdditionalObjects({ type: 'import-dropped' }, files)
      } else {
        console.info(
          '[octos mock] dropped',
          files.map((file) => file.name),
        )
      }
    }
    window.addEventListener('dragover', onDragOver)
    window.addEventListener('dragleave', onDragLeave)
    window.addEventListener('drop', onDrop)
    return () => {
      window.removeEventListener('dragover', onDragOver)
      window.removeEventListener('dragleave', onDragLeave)
      window.removeEventListener('drop', onDrop)
    }
  }, [])

  useEffect(() => {
    if (!notice) {
      return
    }
    const timer = window.setTimeout(
      () => setNotice(null),
      notice.tone === 'error' ? 7000 : 4000,
    )
    return () => window.clearTimeout(timer)
  }, [notice])

  useEffect(() => {
    if (!isHosted()) {
      return
    }

    send({ type: 'ping', id: Date.now() })
    send({ type: 'get-desktop-icons' })
    send({ type: 'get-settings' })
    send({ type: 'get-library' })

    return subscribe((message) => {
      if (message.type === 'pong') {
        setVersion(message.version)
        setHostState('connected')
      } else if (message.type === 'wallpaper-state') {
        setWallpaperActive(message.active)
      } else if (message.type === 'desktop-icons') {
        setDesktopIcons(message.visible)
      } else if (message.type === 'settings') {
        setSettings(message.data)
      } else if (message.type === 'library') {
        const wasInstalling = installingRef.current
        const baseline = installBaselineRef.current
        setInstalling(null)
        const snapshot = message.data
        setLibrary(snapshot)
        if (wasInstalling && snapshot.mods.length > baseline) {
          setNotice({ message: t('library.installDone'), tone: 'success' })
        }
        setSelectedModPath((current) => {
          if (current && snapshot.mods.some((mod) => mod.folder_path === current)) {
            return current
          }
          // On first load, prefer the wallpaper that is actually on screen.
          const activePath = snapshot.monitors
            .map((monitor) => snapshot.selected[monitor])
            .find((path): path is string => Boolean(path))
          if (activePath && snapshot.mods.some((mod) => mod.folder_path === activePath)) {
            return activePath
          }
          return snapshot.mods[0]?.folder_path ?? null
        })
        setSelectedMonitor((current) =>
          current && snapshot.monitors.includes(current)
            ? current
            : (snapshot.monitors[0] ?? ''),
        )
      } else if (message.type === 'notice') {
        setInstalling(null)
        setNotice({ message: message.message, tone: 'error' })
      } else if (message.type === 'error') {
        setInstalling(null)
        setNotice({ message: message.message, tone: 'error' })
      }
    })
  }, [])

  function pingHost() {
    if (!isHosted()) {
      setHostState('preview')
      return
    }
    setHostState('waiting')
    send({ type: 'ping', id: Date.now() })
  }

  function showGradient() {
    if (isHosted()) {
      send({ type: 'show-wallpaper' })
    }
  }

  function clearWallpaper() {
    if (isHosted()) {
      send({ type: 'clear-wallpaper' })
    }
  }

  function installMod() {
    if (!isHosted() || installing) {
      return
    }
    installBaselineRef.current = library?.mods.length ?? 0
    setInstalling('mod')
    send({ type: 'install-mod' })
  }

  function installFolder() {
    if (!isHosted() || installing) {
      return
    }
    installBaselineRef.current = library?.mods.length ?? 0
    setInstalling('folder')
    send({ type: 'install-folder' })
  }

  function installImage() {
    if (!isHosted() || installing) {
      return
    }
    installBaselineRef.current = library?.mods.length ?? 0
    setInstalling('image')
    send({ type: 'install-image' })
  }

  function removeMod(folderPath: string, name: string) {
    if (!isHosted()) {
      return
    }
    if (window.confirm(t('library.removeConfirm', { name }))) {
      send({ type: 'remove-mod', folder_path: folderPath })
    }
  }

  function applySelectedMod() {
    if (!isHosted() || !selectedModPath || !selectedMonitor) {
      return
    }
    send({
      type: 'set-monitor-mod',
      monitor_id: selectedMonitor,
      folder_path: selectedModPath,
    })
  }

  function changeOption(folderPath: string, id: string, value: unknown) {
    if (isHosted()) {
      send({ type: 'set-option', folder_path: folderPath, id, value })
    }
  }

  function setSetting(key: keyof SettingsSnapshot, value: boolean | number) {
    if (isHosted()) {
      send({ type: 'set-setting', key, value })
    } else {
      setSettings((current) => ({ ...current, [key]: value }) as SettingsSnapshot)
    }
  }

  function toggleDesktopIcons() {
    const next = !desktopIcons
    if (isHosted()) {
      send({ type: 'set-desktop-icons', visible: next })
    } else {
      setDesktopIcons(next)
    }
  }

  const selectedCard = library?.mods.find((card) => card.folder_path === selectedModPath) ?? null
  const selectedOptions = selectedCard
    ? (library?.options[selectedCard.folder_path] ?? selectedCard.options)
    : {}
  const loadingLibrary = isHosted() && library === null

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark">O</div>
          <div>
            <strong>{t('app.name')}</strong>
            <small>{t('app.tagline')}</small>
          </div>
        </div>

        <nav className="nav" aria-label={t('nav.primary')}>
          <button
            className={tab === 'library' ? 'nav-item active' : 'nav-item'}
            onClick={() => setTab('library')}
          >
            <LibraryIcon />
            <span>{t('nav.library')}</span>
          </button>
          <button
            className={tab === 'settings' ? 'nav-item active' : 'nav-item'}
            onClick={() => setTab('settings')}
          >
            <SettingsIcon />
            <span>{t('nav.settings')}</span>
          </button>
        </nav>

        <div className="sidebar-footer">
          <span className={`status-dot ${hostState}`} />
          <span>{isHosted() ? t('common.rustHost') : t('common.previewMode')}</span>
          <span className="badge">M4</span>
        </div>
      </aside>

      <main>
        <header className="page-header">
          <div>
            <p className="eyebrow">
              {tab === 'library' ? t('library.eyebrow') : t('settings.eyebrow')}
            </p>
            <h1>{tab === 'library' ? t('library.title') : t('settings.title')}</h1>
            <p className="subhead">
              {tab === 'library' ? t('library.subtitle') : t('settings.subtitle')}
            </p>
            {tab === 'library' && (
              <>
                {library && (
                  <p className="library-meta">
                    {t('library.stats', {
                      mods: library.mods.length,
                      monitors: library.monitors.length,
                    })}
                  </p>
                )}
                <p className="drop-hint">{t('library.dropHint')}</p>
              </>
            )}
          </div>
          {tab === 'library' && (
          <div className="header-actions">
            <button
              className="button"
              onClick={installImage}
              disabled={installing !== null}
              aria-busy={installing === 'image'}
              title={t('library.addImageHint')}
            >
              {installing === 'image' ? <span className="spinner" /> : <ImageIcon />}
              <span>
                {installing === 'image' ? t('library.installing') : t('library.addImage')}
              </span>
            </button>
            <button
              className="button"
              onClick={installMod}
              disabled={installing !== null}
              aria-busy={installing === 'mod'}
              title={t('library.installSoon')}
            >
              {installing === 'mod' ? <span className="spinner" /> : <DownloadIcon />}
              <span>
                {installing === 'mod' ? t('library.installing') : t('library.install')}
              </span>
            </button>
            <button
              className="button"
              onClick={installFolder}
              disabled={installing !== null}
              aria-busy={installing === 'folder'}
              title={t('library.importFolderHint')}
            >
              {installing === 'folder' ? <span className="spinner" /> : <FolderPlusIcon />}
              <span>
                {installing === 'folder'
                  ? t('library.installing')
                  : t('library.importFolder')}
              </span>
            </button>
            <button
              className="button primary"
              onClick={pingHost}
              title={t('common.pingHost')}
            >
              <RefreshIcon />
              <span>{t('common.pingHost')}</span>
            </button>
          </div>
          )}
        </header>

        {tab === 'library' ? (
          <>
            <section className="content-grid" aria-busy={loadingLibrary}>
              {loadingLibrary ? (
                <>
                  <SkeletonGrid />
                  <div className="mod-detail-stack">
                    <DetailSkeleton />
                  </div>
                </>
              ) : library && library.mods.length > 0 ? (
                <>
                  <ModGrid
                    mods={library.mods}
                    selectedPath={selectedModPath}
                    onSelect={setSelectedModPath}
                  />
                  <div className="mod-detail-stack">
                    {selectedCard && (
                      <ModDetail
                        card={selectedCard}
                        options={selectedOptions}
                        monitors={library.monitors}
                        selectedMonitor={selectedMonitor}
                        selected={library.selected}
                        mods={library.mods}
                        onMonitorChange={setSelectedMonitor}
                        onApply={applySelectedMod}
                        onRemove={() =>
                          removeMod(selectedCard.folder_path, selectedCard.name)
                        }
                        onOptionChange={(id, value) =>
                          changeOption(selectedCard.folder_path, id, value)
                        }
                      />
                    )}
                    {library.errors.length > 0 && (
                      <p className="warn-text">{library.errors.join(' · ')}</p>
                    )}
                    <StatusCard
                      state={hostState}
                      version={version}
                      wallpaperActive={wallpaperActive}
                    />
                  </div>
                </>
              ) : (
                <>
                  <EmptyLibrary
                    status={hostState}
                    active={wallpaperActive}
                    onShow={showGradient}
                    onClear={clearWallpaper}
                    onInstall={installMod}
                    onImport={installFolder}
                    onAddImage={installImage}
                  />
                  <StatusCard
                    state={hostState}
                    version={version}
                    wallpaperActive={wallpaperActive}
                  />
                </>
              )}
            </section>
          </>
        ) : (
          <SettingsView
            desktopIcons={desktopIcons}
            onToggleDesktopIcons={toggleDesktopIcons}
            settings={settings}
            onSetSetting={setSetting}
          />
        )}

        <footer className="footer">
          <span>{t('footer.left')}</span>
          <span>{t('footer.right')}</span>
        </footer>
      </main>

      {notice && (
        <div
          className={`toast ${notice.tone}`}
          role={notice.tone === 'error' ? 'alert' : 'status'}
        >
          <span className="toast-icon" aria-hidden="true">
            {notice.tone === 'error' ? <WarningIcon /> : <InfoIcon />}
          </span>
          <span className="toast-message">{notice.message}</span>
          <button
            type="button"
            className="toast-close"
            onClick={() => setNotice(null)}
            aria-label={t('common.dismiss')}
          >
            ×
          </button>
        </div>
      )}

      {dragging && (
        <div className="drop-overlay">
          <div className="drop-card">
            <span className="drop-icon" aria-hidden="true">
              <DownloadIcon />
            </span>
            <strong>{t('library.dropTitle')}</strong>
            <span>{t('library.dropBody')}</span>
          </div>
        </div>
      )}
    </div>
  )
}
