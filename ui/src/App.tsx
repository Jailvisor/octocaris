import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'
import type { TFunction } from 'i18next'
import {
  isHosted,
  send,
  subscribe,
  type LibrarySnapshot,
  type ModCard,
  type PlaylistSnapshot,
  type SettingsSnapshot,
  type OptionSpec,
} from './bridge'
import { setLanguage, type AppLanguage } from './i18n'
import logoUrl from './assets/octocaris.svg'

type Tab = 'library' | 'settings'
type NoticeTone = 'error' | 'success' | 'info'
type Notice = { message: string; tone: NoticeTone }

const DEFAULT_PLAYLIST: PlaylistSnapshot = {
  enabled: false,
  items: [],
  intervalSeconds: 30 * 60,
  random: false,
  current: null,
  missing: [],
}

function splitInterval(seconds: number): { minutes: number; seconds: number } {
  const total = Math.max(0, Math.floor(seconds))
  return { minutes: Math.floor(total / 60), seconds: total % 60 }
}

function monitorLabel(monitor: string, names?: Record<string, string>) {
  const name = names?.[monitor]?.trim()
  return name ? name : prettyMonitor(monitor)
}

function monitorOptions(monitors: string[], names?: Record<string, string>) {
  const labels = monitors.map((monitor) => monitorLabel(monitor, names))
  return monitors.map((monitor, index) => {
    const label = labels[index]
    const duplicate = labels.filter((other) => other === label).length > 1
    return {
      value: monitor,
      label: duplicate ? `${label} · ${prettyMonitor(monitor)}` : label,
    }
  })
}

const CARD_DRAG_MIME = 'application/x-octos-card'

function isCardDrag(dataTransfer: DataTransfer | null) {
  return Boolean(dataTransfer && Array.from(dataTransfer.types).includes(CARD_DRAG_MIME))
}

/** Insert `items[from]` before original index `insertAt` (0..=len). */
function moveBefore<T>(items: T[], from: number, insertAt: number): T[] {
  if (from < 0 || from >= items.length) {
    return items
  }
  const target = from < insertAt ? insertAt - 1 : insertAt
  if (target < 0 || target >= items.length || target === from) {
    return items
  }
  const next = items.slice()
  const [item] = next.splice(from, 1)
  next.splice(target, 0, item)
  return next
}

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
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
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
      <path d="M20 11a8 8 0 1 0-2.3 5.7" />
      <path d="M20 5v6h-6" />
    </svg>
  )
}

function EyeIcon({ open }: { open: boolean }) {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M2 12s3.5-6.5 10-6.5S22 12 22 12s-3.5 6.5-10 6.5S2 12 2 12Z" />
      <circle cx="12" cy="12" r="2.8" />
      {!open && <path d="M4 4l16 16" />}
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

function EmptyLibrary({
  active,
  onShow,
  onClear,
  onInstall,
  onImport,
  onAddImage,
}: {
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
          {t('common.showDefault')}
        </button>
        <button className="button ghost" onClick={onClear} disabled={!active}>
          {t('common.clearWallpaper')}
        </button>
      </div>
    </div>
  )
}

function prettyMonitor(id: string) {
  return id.replace(/^\\\\.\\/, '') || id
}

const KNOWN_OPTION_LABELS: Record<string, string> = {
  fit: 'library.optionLabel.fit',
}

const KNOWN_OPTION_VALUES: Record<string, Record<string, string>> = {
  fit: {
    cover: 'library.optionValue.cover',
    contain: 'library.optionValue.contain',
    fill: 'library.optionValue.fill',
  },
}

function translateOptionLabel(id: string, fallback: string, t: TFunction) {
  const key = KNOWN_OPTION_LABELS[id]
  if (!key) {
    return fallback
  }
  const translated = t(key)
  return translated === key ? fallback : translated
}

function translateOptionValue(id: string, option: string, t: TFunction) {
  const key = KNOWN_OPTION_VALUES[id]?.[option]
  if (!key) {
    return option
  }
  const translated = t(key)
  return translated === key ? option : translated
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
  const { t } = useTranslation()
  const type = String(spec.type ?? 'checkbox')
  const fallbackLabel = typeof spec.label === 'string' && spec.label ? spec.label : id
  const label = translateOptionLabel(id, fallbackLabel, t)
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
              {translateOptionValue(id, option, t)}
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
  monitorNames,
  selectedMonitor,
  selected,
  onMonitorChange,
  onApply,
  onRemove,
  onOptionChange,
  playlistEnabled,
}: {
  card: ModCard
  options: Record<string, OptionSpec>
  monitors: string[]
  monitorNames?: Record<string, string>
  selectedMonitor: string
  selected: Record<string, string>
  onMonitorChange: (monitor: string) => void
  onApply: () => void
  onRemove: () => void
  onOptionChange: (id: string, value: unknown) => void
  playlistEnabled: boolean
}) {
  const { t } = useTranslation()
  const [confirmRemove, setConfirmRemove] = useState(false)
  const entries = Object.entries(options)
  const inUse = Object.values(selected).includes(card.folder_path)

  useEffect(() => setConfirmRemove(false), [card.folder_path])

  return (
    <div className="panel mod-detail">
      <div className="detail-heading">
        <h2>{card.name}</h2>
        {inUse && (
          <span className="pill success">
            <span className="status-dot connected" />
            {t('library.inUse')}
          </span>
        )}
      </div>
      {!card.entry_url && <p className="warn-text">{t('library.notRunnable')}</p>}

      <div className="apply-row">
        <label>
          {t('library.monitor')}
          <select
            value={selectedMonitor}
            onChange={(event) => onMonitorChange(event.target.value)}
            disabled={monitors.length === 0}
          >
            {monitorOptions(monitors, monitorNames).map((option) => (
              <option key={option.value} value={option.value} title={option.value}>
                {option.label}
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
      </div>

      <div className="detail-actions">
        {confirmRemove ? (
          <span className="confirm-inline">
            <span className="tiny">{t('library.removeConfirm')}</span>
            <button className="button ghost danger-text" onClick={onRemove}>
              {t('library.confirm')}
            </button>
            <button className="button ghost" onClick={() => setConfirmRemove(false)}>
              {t('library.cancel')}
            </button>
          </span>
        ) : (
          <button className="button ghost quiet-danger" onClick={() => setConfirmRemove(true)}>
            {t('library.remove')}
          </button>
        )}
      </div>
      {playlistEnabled && <p className="tiny playlist-hint">{t('playlist.overrideHint')}</p>}

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

function SelectAllIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M3.5 7.5l2 2 3.5-3.5" />
      <path d="M3.5 17.5l2 2 3.5-3.5" />
      <path d="M13.5 8h7M13.5 18h7" />
    </svg>
  )
}

function NextIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M5 5.5l9 6.5-9 6.5v-13z" />
      <path d="M19 5v14" />
    </svg>
  )
}

function DragGripIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="9" cy="6" r="1.7" />
      <circle cx="15" cy="6" r="1.7" />
      <circle cx="9" cy="12" r="1.7" />
      <circle cx="15" cy="12" r="1.7" />
      <circle cx="9" cy="18" r="1.7" />
      <circle cx="15" cy="18" r="1.7" />
    </svg>
  )
}

function ModCardTile({
  card,
  index,
  selected,
  tabbable,
  inPlaylist,
  dragging,
  dropBefore,
  dropAfter,
  onSelect,
  onToggle,
  onDragStart,
  onDragOver,
  onDrop,
  onDragEnd,
}: {
  card: ModCard
  index: number
  selected: boolean
  tabbable: boolean
  inPlaylist: boolean
  dragging: boolean
  dropBefore: boolean
  dropAfter: boolean
  onSelect: (path: string) => void
  onToggle: (path: string) => void
  onDragStart: (index: number) => void
  onDragOver: (index: number, after: boolean) => void
  onDrop: (index: number, after: boolean) => void
  onDragEnd: () => void
}) {
  const { t } = useTranslation()
  const [imageState, setImageState] = useState<'idle' | 'ready' | 'error'>(
    card.image_url ? 'idle' : 'error',
  )
  const dragOriginIsInput = useRef(false)

  useEffect(() => {
    setImageState(card.image_url ? 'idle' : 'error')
  }, [card.image_url])

  const wrapClass = [
    'mod-card-wrap',
    dragging ? 'dragging' : '',
    dropBefore ? 'drop-before' : '',
    dropAfter ? 'drop-after' : '',
  ]
    .filter(Boolean)
    .join(' ')

  return (
    <div
      className={wrapClass}
      draggable
      data-path={card.folder_path}
      onMouseDown={(event) => {
        dragOriginIsInput.current = Boolean((event.target as Element).closest('input'))
      }}
      onDragStart={(event) => {
        if (dragOriginIsInput.current) {
          event.preventDefault()
          return
        }
        event.dataTransfer.effectAllowed = 'move'
        event.dataTransfer.setData(CARD_DRAG_MIME, card.folder_path)
        onDragStart(index)
      }}
      onDragOver={(event) => {
        if (!isCardDrag(event.dataTransfer)) {
          return
        }
        event.preventDefault()
        event.stopPropagation()
        event.dataTransfer.dropEffect = 'move'
        const rect = event.currentTarget.getBoundingClientRect()
        onDragOver(index, event.clientX > rect.left + rect.width / 2)
      }}
      onDrop={(event) => {
        if (!isCardDrag(event.dataTransfer)) {
          return
        }
        event.preventDefault()
        event.stopPropagation()
        const rect = event.currentTarget.getBoundingClientRect()
        onDrop(index, event.clientX > rect.left + rect.width / 2)
      }}
      onDragEnd={onDragEnd}
    >
      <button
        type="button"
        role="option"
        aria-selected={selected}
        tabIndex={tabbable ? 0 : -1}
        data-mod-card
        data-index={index}
        data-path={card.folder_path}
        className={selected ? 'mod-card selected' : 'mod-card'}
        onClick={() => onSelect(card.folder_path)}
        onFocus={() => onSelect(card.folder_path)}
      >
        <div className="mod-card-art">
          {card.image_url && imageState !== 'error' && (
            <img
              src={card.image_url}
              alt=""
              draggable={false}
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
        </div>
        <div className="mod-card-body">
          <h3>{card.name}</h3>
        </div>
      </button>
      <input
        type="checkbox"
        className="rotation-check"
        checked={inPlaylist}
        disabled={!card.entry_url}
        aria-label={inPlaylist ? t('playlist.removeFrom') : t('playlist.addTo')}
        title={inPlaylist ? t('playlist.removeFrom') : t('playlist.addTo')}
        onChange={() => onToggle(card.folder_path)}
      />
      <span className="drag-grip" aria-hidden="true">
        <DragGripIcon />
      </span>
    </div>
  )
}

function ModGrid({
  mods,
  selectedPath,
  playlistPaths,
  onSelect,
  onToggle,
  onReorder,
}: {
  mods: ModCard[]
  selectedPath: string | null
  playlistPaths: string[]
  onSelect: (path: string) => void
  onToggle: (path: string) => void
  onReorder: (from: number, insertAt: number) => void
}) {
  const { t } = useTranslation()
  const gridRef = useRef<HTMLDivElement>(null)
  const dragIndexRef = useRef<number | null>(null)
  const [dragIndex, setDragIndex] = useState<number | null>(null)
  const [overIndex, setOverIndex] = useState<number | null>(null)
  const [dropAfter, setDropAfter] = useState(false)

  function resetDrag() {
    dragIndexRef.current = null
    setDragIndex(null)
    setOverIndex(null)
  }

  function handleDragStart(index: number) {
    dragIndexRef.current = index
    // Defer the faded class so Chromium does not snapshot it as the drag image.
    requestAnimationFrame(() => setDragIndex(index))
  }

  function handleDrop(target: number, after: boolean) {
    const from = dragIndexRef.current
    if (from !== null) {
      onReorder(from, target + (after ? 1 : 0))
    }
    resetDrag()
  }

  function handleKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    const grid = gridRef.current
    if (!grid) {
      return
    }
    const cards = Array.from(grid.querySelectorAll<HTMLButtonElement>('[data-mod-card]'))
    const active = document.activeElement as HTMLElement | null
    const activeCard =
      active?.closest<HTMLElement>('.mod-card-wrap')?.querySelector<HTMLButtonElement>('[data-mod-card]') ??
      (active as HTMLButtonElement | null)
    const current = activeCard ? cards.indexOf(activeCard) : -1
    if (current < 0) {
      return
    }

    if (event.altKey) {
      const delta = event.key === 'ArrowUp' ? -1 : event.key === 'ArrowDown' ? 1 : 0
      if (delta === 0) {
        return
      }
      event.preventDefault()
      const path = mods[current]?.folder_path
      onReorder(current, delta < 0 ? current - 1 : current + 2)
      requestAnimationFrame(() => {
        const moved = Array.from(
          gridRef.current?.querySelectorAll<HTMLButtonElement>('[data-mod-card]') ?? [],
        ).find((node) => node.dataset.path === path)
        moved?.focus()
      })
      return
    }

    const keys = ['ArrowRight', 'ArrowLeft', 'ArrowDown', 'ArrowUp', 'Home', 'End']
    if (!keys.includes(event.key)) {
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
      className={dragIndex === null ? 'mod-grid' : 'mod-grid is-dragging'}
      role="listbox"
      aria-label={t('library.title')}
      title={t('library.keyboardHint')}
      onKeyDown={handleKeyDown}
      onDragOver={(event) => {
        if (!isCardDrag(event.dataTransfer)) {
          return
        }
        event.preventDefault()
        event.dataTransfer.dropEffect = 'move'
        setOverIndex(null)
      }}
      onDrop={(event) => {
        if (!isCardDrag(event.dataTransfer)) {
          return
        }
        event.preventDefault()
        const from = dragIndexRef.current
        if (from !== null) {
          onReorder(from, mods.length)
        }
        resetDrag()
      }}
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
          inPlaylist={playlistPaths.includes(card.folder_path)}
          dragging={dragIndex === index}
          dropBefore={
            dragIndex !== null && overIndex === index && !dropAfter && dragIndex !== index
          }
          dropAfter={
            dragIndex !== null && overIndex === index && dropAfter && dragIndex !== index
          }
          onSelect={onSelect}
          onToggle={onToggle}
          onDragStart={handleDragStart}
          onDragOver={(target, after) => {
            setOverIndex(target)
            setDropAfter(after)
          }}
          onDrop={handleDrop}
          onDragEnd={resetDrag}
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
  disabled = false,
  onChange,
}: {
  checked: boolean
  label: string
  disabled?: boolean
  onChange: (value: boolean) => void
}) {
  return (
    <button
      className={checked ? 'switch interactive active' : 'switch interactive'}
      role="switch"
      aria-checked={checked}
      aria-disabled={disabled}
      aria-label={label}
      disabled={disabled}
      onClick={() => {
        if (!disabled) {
          onChange(!checked)
        }
      }}
    >
      <span />
    </button>
  )
}

function SettingsView({
  desktopIcons,
  onToggleDesktopIcons,
  onOpenFolder,
  settings,
  onSetSetting,
}: {
  desktopIcons: boolean
  onToggleDesktopIcons: () => void
  onOpenFolder: () => void
  settings: SettingsSnapshot
  onSetSetting: (key: keyof SettingsSnapshot, value: boolean | number) => void
}) {
  const { t, i18n } = useTranslation()
  const language: AppLanguage = i18n.language.toLowerCase().startsWith('zh') ? 'zh-CN' : 'en'
  return (
    <div className="panel settings-panel">
      <div className="setting-row">
        <div>
          <h3>{t('settings.showDesktopIcons.title')}</h3>
          <p>{t('settings.showDesktopIcons.desc')}</p>
        </div>
        <div className="setting-tail">
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
          <h3>{t('settings.openFolder.title')}</h3>
          <p>{t('settings.openFolder.desc')}</p>
        </div>
        <div className="setting-tail">
          <button type="button" className="button" onClick={onOpenFolder}>
            {t('settings.openFolder.action')}
          </button>
        </div>
      </div>
      <div className="setting-row">
        <div>
          <h3>{t('settings.language.title')}</h3>
          <p>{t('settings.language.desc')}</p>
        </div>
        <div className="setting-tail">
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
          <SettingSwitch
            checked={settings.runOnStartup}
            label={t('settings.startWithWindows.title')}
            onChange={(value) => onSetSetting('runOnStartup', value)}
          />
        </div>
      </div>
      <div className="setting-row">
        <div>
          <h3>{t('settings.memorySaver.title')}</h3>
          <p>{t('settings.memorySaver.desc')}</p>
        </div>
        <div className="setting-tail">
          <SettingSwitch
            checked={settings.memorySaver}
            label={t('settings.memorySaver.title')}
            onChange={(value) => onSetSetting('memorySaver', value)}
          />
        </div>
      </div>
      <div className="setting-row">
        <div>
          <h3>{t('settings.maxFps.title')}</h3>
          <p>{t('settings.maxFps.desc')}</p>
        </div>
        <div className="setting-tail">
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
          <SettingSwitch
            checked={settings.enableGpu}
            label={t('settings.enableGpu.title')}
            onChange={(value) => onSetSetting('enableGpu', value)}
          />
        </div>
      </div>
    </div>
  )
}

function PlaylistPanel({
  playlist,
  mods,
  onToggleEnabled,
  onSetInterval,
  onSetRandom,
  onSelectAll,
  onNext,
}: {
  playlist: PlaylistSnapshot
  mods: ModCard[]
  onToggleEnabled: () => void
  onSetInterval: (seconds: number) => void
  onSetRandom: (random: boolean) => void
  onSelectAll: () => void
  onNext: () => void
}) {
  const { t } = useTranslation()
  const initialInterval = splitInterval(playlist.intervalSeconds)
  const [intervalMinutes, setIntervalMinutes] = useState(initialInterval.minutes)
  const [intervalSeconds, setIntervalSeconds] = useState(initialInterval.seconds)

  useEffect(() => {
    const parts = splitInterval(playlist.intervalSeconds)
    setIntervalMinutes(parts.minutes)
    setIntervalSeconds(parts.seconds)
  }, [playlist.intervalSeconds])

  if (mods.length === 0) {
    return (
      <section className="panel playlist-panel playlist-empty">
        <p className="tiny">{t('playlist.emptyHint')}</p>
      </section>
    )
  }

  const items = playlist.items
  const runnable = mods.filter((card) => card.entry_url)
  const status = !playlist.enabled
    ? t('playlist.status.inactive')
    : items.length < 2
      ? t('playlist.status.needTwo')
      : playlist.missing.length > 0
        ? t('playlist.status.missing', { count: playlist.missing.length })
        : t('playlist.status.active', { count: items.length })

  function commitInterval(minutes: number, seconds: number) {
    const safeMinutes = Number.isFinite(minutes) ? Math.max(0, Math.floor(minutes)) : 0
    const safeSeconds = Number.isFinite(seconds) ? Math.min(59, Math.max(0, Math.floor(seconds))) : 0
    onSetInterval(safeMinutes * 60 + safeSeconds)
  }

  return (
    <section className="panel playlist-panel" aria-label={t('playlist.panelTitle')}>
      <div className="playlist-head">
        <div>
          <h2>{t('playlist.panelTitle')}</h2>
          <p className="tiny">{status}</p>
        </div>
        <div className="setting-tail">
          <button
            type="button"
            className={
              playlist.enabled && items.length > 0
                ? 'switch interactive active'
                : 'switch interactive'
            }
            role="switch"
            aria-checked={playlist.enabled && items.length > 0}
            aria-label={t('playlist.enable.title')}
            disabled={items.length === 0}
            onClick={onToggleEnabled}
          >
            <span />
          </button>
        </div>
      </div>

      <div className="playlist-controls">
        <div className="playlist-control">
          <span>{t('playlist.interval.title')}</span>
          <div className="interval-inputs">
            <input
              type="number"
              min={0}
              step={1}
              value={intervalMinutes}
              aria-label={t('playlist.interval.unitMinutes')}
              onChange={(event) => {
                const next = Number(event.target.value)
                setIntervalMinutes(next)
                commitInterval(next, intervalSeconds)
              }}
            />
            <span>{t('playlist.interval.unitMinutes')}</span>
            <input
              type="number"
              min={0}
              max={59}
              step={1}
              value={intervalSeconds}
              aria-label={t('playlist.interval.unitSeconds')}
              onChange={(event) => {
                const next = Number(event.target.value)
                setIntervalSeconds(next)
                commitInterval(intervalMinutes, next)
              }}
            />
            <span>{t('playlist.interval.unitSeconds')}</span>
          </div>
        </div>

        <div className="playlist-control">
          <span>{t('playlist.random.title')}</span>
          <button
            type="button"
            className={playlist.random ? 'switch interactive active' : 'switch interactive'}
            role="switch"
            aria-checked={playlist.random}
            aria-label={t('playlist.random.title')}
            onClick={() => onSetRandom(!playlist.random)}
          >
            <span />
          </button>
        </div>
      </div>

      <div className="playlist-footer">
        <button
          type="button"
          className="button"
          onClick={onSelectAll}
          disabled={
            runnable.length === 0 ||
            runnable.every((card) => items.includes(card.folder_path))
          }
        >
          <SelectAllIcon />
          <span>{t('playlist.selectAll')}</span>
        </button>
        <button
          type="button"
          className="button"
          onClick={onNext}
          disabled={items.length < 2}
        >
          <NextIcon />
          <span>{t('playlist.next')}</span>
        </button>
      </div>
    </section>
  )
}

export default function App() {
  const { t } = useTranslation()
  const [tab, setTab] = useState<Tab>('library')
  const [wallpaperActive, setWallpaperActive] = useState(true)
  const [desktopIcons, setDesktopIcons] = useState(true)
  const [wallpaperVisible, setWallpaperVisible] = useState(true)
  const [settings, setSettings] = useState<SettingsSnapshot>({
    runOnStartup: false,
    enableGpu: true,
    pauseFullscreen: true,
    maxFps: 30,
    memorySaver: true,
  })
  const [library, setLibrary] = useState<LibrarySnapshot | null>(null)
  const [selectedModPath, setSelectedModPath] = useState<string | null>(null)
  const [selectedMonitor, setSelectedMonitor] = useState('')
  const [installing, setInstalling] = useState<'mod' | 'folder' | 'image' | null>(null)
  const [dragging, setDragging] = useState(false)
  const [notice, setNotice] = useState<Notice | null>(null)
  const installingRef = useRef<'mod' | 'folder' | 'image' | null>(null)
  const installBaselineRef = useRef(0)
  const refreshPendingRef = useRef(false)

  useEffect(() => {
    installingRef.current = installing
  }, [installing])

  useEffect(() => {
    if (tab !== 'library') {
      return
    }
    function onKeyDown(event: KeyboardEvent) {
      if (!(event.ctrlKey || event.metaKey)) {
        return
      }
      const target = event.target as HTMLElement | null
      if (
        target &&
        (target.tagName === 'INPUT' ||
          target.tagName === 'TEXTAREA' ||
          target.tagName === 'SELECT' ||
          target.isContentEditable)
      ) {
        return
      }
      const key = event.key.toLowerCase()
      if (key === 'a') {
        event.preventDefault()
        addAllWallpapers()
      } else if (key === 'd') {
        event.preventDefault()
        clearPlaylist()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  })

  useEffect(() => {
    function onDragStart(event: DragEvent) {
      const target = event.target
      if (target instanceof Element && target.closest('.mod-card-wrap')) {
        return
      }
      event.preventDefault()
    }
    window.addEventListener('dragstart', onDragStart)
    return () => window.removeEventListener('dragstart', onDragStart)
  }, [])

  useEffect(() => {
    function onDragOver(event: DragEvent) {
      if (!event.dataTransfer || isCardDrag(event.dataTransfer)) {
        return
      }
      if (!event.dataTransfer.types.includes('Files')) {
        return
      }
      event.preventDefault()
      event.dataTransfer.dropEffect = 'copy'
      setDragging(true)
    }
    function onDragLeave(event: DragEvent) {
      if (event.relatedTarget === null) {
        setDragging(false)
      }
    }
    function onDrop(event: DragEvent) {
      if (isCardDrag(event.dataTransfer)) {
        return
      }
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
    send({ type: 'get-wallpaper-visibility' })
    send({ type: 'get-settings' })
    send({ type: 'get-library' })

    return subscribe((message) => {
      if (message.type === 'wallpaper-state') {
        setWallpaperActive(message.active)
      } else if (message.type === 'desktop-icons') {
        setDesktopIcons(message.visible)
      } else if (message.type === 'wallpaper-visibility') {
        setWallpaperVisible(message.visible)
      } else if (message.type === 'settings') {
        setSettings(message.data)
      } else if (message.type === 'library') {
        if (refreshPendingRef.current) {
          refreshPendingRef.current = false
          setNotice({ message: t('library.refreshed'), tone: 'success' })
        }
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
        refreshPendingRef.current = false
        setInstalling(null)
        setNotice({ message: message.message, tone: 'error' })
      }
    })
  }, [])

  function showDefaultWallpaper() {
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

  function removeMod(folderPath: string) {
    if (isHosted()) {
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

  function toggleWallpaperVisibility() {
    const visible = !wallpaperVisible
    if (isHosted()) {
      send({ type: 'set-wallpaper-visibility', visible })
    } else {
      setWallpaperVisible(visible)
    }
  }

  function openWallpapersFolder() {
    if (isHosted()) {
      send({ type: 'open-wallpapers-folder' })
    }
  }

  function refreshLibrary() {
    if (isHosted()) {
      refreshPendingRef.current = true
      send({ type: 'refresh-library' })
    } else {
      setNotice({ message: t('library.refreshed'), tone: 'success' })
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

  const playlist = library?.playlist ?? DEFAULT_PLAYLIST

  function mutatePlaylist(message: Record<string, unknown>, patch: Partial<PlaylistSnapshot>) {
    if (isHosted()) {
      send(message)
    } else {
      setLibrary((current) =>
        current
          ? {
              ...current,
              playlist: { ...(current.playlist ?? DEFAULT_PLAYLIST), ...patch },
            }
          : current,
      )
    }
  }

  function setPlaylistItems(items: string[]) {
    mutatePlaylist({ type: 'set-playlist-items', items }, { items })
  }

  function reorderLibrary(from: number, insertAt: number) {
    if (!library) {
      return
    }
    const mods = moveBefore(library.mods, from, insertAt)
    if (mods === library.mods) {
      return
    }
    setLibrary({ ...library, mods })
    if (isHosted()) {
      send({
        type: 'set-library-order',
        items: mods.map((card) => card.folder_path),
      })
    }
  }

  function togglePlaylistItem(folderPath: string) {
    setPlaylistItems(
      playlist.items.includes(folderPath)
        ? playlist.items.filter((item) => item !== folderPath)
        : [...playlist.items, folderPath],
    )
  }

  function clearPlaylist() {
    setPlaylistItems([])
  }

  function addAllWallpapers() {
    if (!library) {
      return
    }
    const next = [...playlist.items]
    for (const card of library.mods) {
      if (card.entry_url && !next.includes(card.folder_path)) {
        next.push(card.folder_path)
      }
    }
    setPlaylistItems(next)
  }

  function togglePlaylistEnabled() {
    const enabled = !playlist.enabled
    mutatePlaylist({ type: 'set-playlist-enabled', enabled }, { enabled })
  }

  function setPlaylistInterval(seconds: number) {
    mutatePlaylist(
      { type: 'set-playlist-interval', seconds },
      { intervalSeconds: seconds },
    )
  }

  function setPlaylistRandom(random: boolean) {
    mutatePlaylist({ type: 'set-playlist-random', random }, { random })
  }

  function playlistNext() {
    if (isHosted()) {
      send({ type: 'playlist-next' })
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
          <div className="brand-mark">
            <img src={logoUrl} alt="" />
          </div>
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
      </aside>

      <main>
        <header className="page-header">
          <div>
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
              type="button"
              className="header-icon"
              onClick={refreshLibrary}
              title={t('library.refresh')}
              aria-label={t('library.refresh')}
            >
              <RefreshIcon />
            </button>
            <button
              type="button"
              className={wallpaperVisible ? 'header-icon' : 'header-icon off'}
              onClick={toggleWallpaperVisibility}
              title={wallpaperVisible ? t('library.hideWallpaper') : t('library.showWallpaper')}
              aria-label={wallpaperVisible ? t('library.hideWallpaper') : t('library.showWallpaper')}
            >
              <EyeIcon open={wallpaperVisible} />
            </button>
          </div>
          )}
        </header>

        {tab === 'library' ? (
          <section className="library-page" aria-busy={loadingLibrary}>
            {loadingLibrary ? (
              <>
                <div className="library-top">
                  <DetailSkeleton />
                  <DetailSkeleton />
                </div>
                <SkeletonGrid />
              </>
            ) : library && library.mods.length > 0 ? (
              <>
                <div className="library-top">
                  <PlaylistPanel
                    playlist={playlist}
                    mods={library.mods}
                    onToggleEnabled={togglePlaylistEnabled}
                    onSetInterval={setPlaylistInterval}
                    onSetRandom={setPlaylistRandom}
                    onSelectAll={addAllWallpapers}
                    onNext={playlistNext}
                  />
                  {selectedCard ? (
                    <ModDetail
                      card={selectedCard}
                      options={selectedOptions}
                      monitors={library.monitors}
                      monitorNames={library.monitorNames}
                      selectedMonitor={selectedMonitor}
                      selected={library.selected}
                      onMonitorChange={setSelectedMonitor}
                      onApply={applySelectedMod}
                      onRemove={() => removeMod(selectedCard.folder_path)}
                      playlistEnabled={playlist.enabled}
                      onOptionChange={(id, value) =>
                        changeOption(selectedCard.folder_path, id, value)
                      }
                    />
                  ) : (
                    <div className="panel mod-detail detail-empty">
                      <p className="tiny">{t('library.detailEmpty')}</p>
                    </div>
                  )}
                </div>
                <ModGrid
                  mods={library.mods}
                  selectedPath={selectedModPath}
                  playlistPaths={playlist.items}
                  onSelect={setSelectedModPath}
                  onToggle={togglePlaylistItem}
                  onReorder={reorderLibrary}
                />
                {library.errors.length > 0 && (
                  <p className="warn-text">{library.errors.join(' · ')}</p>
                )}
              </>
            ) : (
              <EmptyLibrary
                active={wallpaperActive}
                onShow={showDefaultWallpaper}
                onClear={clearWallpaper}
                onInstall={installMod}
                onImport={installFolder}
                onAddImage={installImage}
              />
            )}
          </section>
        ) : (
          <SettingsView
            desktopIcons={desktopIcons}
            onToggleDesktopIcons={toggleDesktopIcons}
            onOpenFolder={openWallpapersFolder}
            settings={settings}
            onSetSetting={setSetting}
          />
        )}
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
