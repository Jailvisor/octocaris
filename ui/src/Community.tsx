import { useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { DetailDrawer } from './DetailDrawer'
import type { CommunityItem, CommunityJob, CommunitySnapshot } from './bridge'

const FALLBACK_PREVIEW = 'https://local.octos/default/image.jpg'

const PAGE_SIZE = 24

export type CommunityActions = {
  onRefresh: () => void
  onInstall: (id: string) => void
  onCancel: () => void
  onOpenPage: (id: string) => void
  onOpenSource: (id: string) => void
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return '0 B'
  }
  const units = ['B', 'KB', 'MB', 'GB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return unit === 0 ? `${value.toFixed(0)} B` : `${value.toFixed(1)} ${units[unit]}`
}

function isActiveJob(job: CommunityJob | null): job is Extract<CommunityJob, { phase: 'waiting' | 'downloading' | 'verifying' | 'installing' }> {
  return job !== null && ['waiting', 'downloading', 'verifying', 'installing'].includes(job.phase)
}

function CommunityCard({
  item,
  onOpen,
}: {
  item: CommunityItem
  onOpen: (folder: string) => void
}) {
  const { t } = useTranslation()
  const [source, setSource] = useState(item.preview_url ?? FALLBACK_PREVIEW)
  const installed = item.installed
  return (
    <button
      type="button"
      className="mod-card community-card"
      data-community-card
      onClick={() => onOpen(item.folder)}
    >
      <div className="mod-card-art">
        <img
          src={source}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          onLoad={(event) => event.currentTarget.classList.add('loaded')}
          onError={() => setSource(FALLBACK_PREVIEW)}
        />
        <span className="community-badges">
          {installed?.update_available && (
            <span className="badge update">{t('community.updateAvailable')}</span>
          )}
          {installed && !installed.update_available && (
            <span className="badge installed">
              {installed.managed ? t('community.installed') : t('community.installedLocal')}
            </span>
          )}
        </span>
      </div>
      <div className="mod-card-body">
        <span className="mod-card-name" title={item.name}>
          {item.name}
        </span>
        <span className="mod-card-meta">
          {item.author || t('community.unknownAuthor')} · {formatBytes(item.download_bytes)}
        </span>
      </div>
    </button>
  )
}

function CommunityDetail({
  item,
  job,
  busy,
  actions,
}: {
  item: CommunityItem
  job: CommunityJob | null
  busy: boolean
  actions: CommunityActions
}) {
  const { t } = useTranslation()
  const [imageFailed, setImageFailed] = useState(false)
  const [showFiles, setShowFiles] = useState(false)
  const [showHash, setShowHash] = useState(false)
  const installed = item.installed
  const jobForItem = job && job.folder === item.folder ? job : null
  const progress =
    jobForItem && (jobForItem.phase === 'downloading' || jobForItem.phase === 'verifying')
      ? jobForItem
      : null
  const percent = progress?.total
    ? Math.min(100, Math.round((progress.received / progress.total) * 100))
    : 0

  useEffect(() => {
    setImageFailed(false)
    setShowFiles(false)
    setShowHash(false)
  }, [item.folder])

  const installedLabel = installed
    ? installed.update_available
      ? t('community.update')
      : t('community.installed')
    : t('community.install')

  return (
    <div className="community-detail">
      <img
        className="drawer-preview"
        src={item.preview_url && !imageFailed ? item.preview_url : FALLBACK_PREVIEW}
        alt=""
        onError={() => setImageFailed(true)}
        draggable={false}
      />

      <h2>{item.name}</h2>
      <p className="tiny">
        {item.author || t('community.unknownAuthor')} · v{item.version} ·{' '}
        {formatBytes(item.download_bytes)}
      </p>
      {item.tags.length > 0 && (
        <div className="community-tags">
          {item.tags.map((tag) => (
            <span className="tag-chip static" key={tag}>
              {tag}
            </span>
          ))}
        </div>
      )}
      {item.description && <p className="community-description">{item.description}</p>}

      {progress && (
        <div className="progress" role="status">
          <div className="progress-bar">
            <span style={{ width: `${percent}%` }} />
          </div>
          <span className="progress-label">
            {progress.phase === 'verifying'
              ? t('community.verifying')
              : t('community.downloading', {
                  received: formatBytes(progress.received),
                  total: progress.total ? formatBytes(progress.total) : '?',
                })}
          </span>
        </div>
      )}
      {jobForItem?.phase === 'waiting' && <p className="tiny">{t('community.waitingIndex')}</p>}
      {jobForItem?.phase === 'installing' && <p className="tiny">{t('community.installing')}</p>}

      <div className="community-actions">
        <button
          type="button"
          className="button primary"
          disabled={busy || (installed !== null && !installed.update_available) || (jobForItem?.phase === 'installing')}
          onClick={() => actions.onInstall(item.folder)}
        >
          {installedLabel}
        </button>
        {progress && (
          <button type="button" className="button ghost" onClick={actions.onCancel}>
            {t('community.cancel')}
          </button>
        )}
      </div>

      <div className="community-links">
        <button
          type="button"
          className="button ghost"
          onClick={() => actions.onOpenPage(item.folder)}
          disabled={!item.page_url}
        >
          {t('community.openOnGitHub')}
        </button>
        {item.workshop_url && (
          <button
            type="button"
            className="button ghost"
            onClick={() => actions.onOpenSource(item.folder)}
          >
            {t('community.openSource')}
          </button>
        )}
      </div>

      <div className="community-meta-list">
        <button type="button" className="link-button" onClick={() => setShowFiles((value) => !value)}>
          {showFiles ? t('community.hideFiles') : t('community.showFiles', { count: item.file_count })}
        </button>
        {showFiles && (
          <ul className="file-list">
            {item.files.map((file) => (
              <li className="file-row" key={file.path}>
                <span className="file-name" title={file.path}>
                  {file.path}
                </span>
                <span className="file-size">{formatBytes(file.bytes)}</span>
              </li>
            ))}
          </ul>
        )}
        {showFiles && item.files_truncated && (
          <p className="tiny">{t('community.filesTruncated')}</p>
        )}
        <button type="button" className="link-button" onClick={() => setShowHash((value) => !value)}>
          {t('community.sha256')}
        </button>
        {showHash && <code className="community-hash">{item.sha256}</code>}
      </div>
    </div>
  )
}

export function CommunityView({
  snapshot,
  actions,
}: {
  snapshot: CommunitySnapshot | null
  actions: CommunityActions
}) {
  const { t } = useTranslation()
  const items = snapshot?.items ?? []
  const [query, setQuery] = useState('')
  const [page, setPage] = useState(0)
  const [selectedFolder, setSelectedFolder] = useState<string | null>(null)

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase()
    const result = items.filter((item) => {
      if (!needle) {
        return true
      }
      return [item.name, item.author, item.description, ...item.tags].some((value) =>
        value.toLowerCase().includes(needle),
      )
    })
    result.sort((left, right) => left.name.localeCompare(right.name))
    return result
  }, [items, query])

  const pageCount = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE))
  const safePage = Math.min(page, pageCount - 1)
  const visible = filtered.slice(safePage * PAGE_SIZE, safePage * PAGE_SIZE + PAGE_SIZE)
  const selected = items.find((item) => item.folder === selectedFolder) ?? null
  const job = snapshot?.job ?? null
  const busy = isActiveJob(job)
  const loading = (snapshot === null || snapshot.status === 'loading') && items.length === 0

  useEffect(() => {
    setPage(0)
  }, [query])

  useEffect(() => {
    if (page > pageCount - 1) {
      setPage(pageCount - 1)
    }
  }, [page, pageCount])

  useEffect(() => {
    if (selectedFolder && !items.some((item) => item.folder === selectedFolder)) {
      setSelectedFolder(null)
    }
  }, [items, selectedFolder])

  return (
    <section className="library-page community-page" aria-busy={loading}>
      <div className="community-toolbar">
        <input
          className="text-input community-search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t('community.searchPlaceholder')}
          aria-label={t('community.searchPlaceholder')}
        />
        <div className="community-toolbar-tail">
          <span className="tiny">
            {t('community.count', { count: filtered.length, total: items.length })}
          </span>
          <button
            type="button"
            className="button ghost"
            onClick={actions.onRefresh}
            disabled={snapshot?.status === 'loading'}
          >
            {t('community.refresh')}
          </button>
        </div>
      </div>

      {snapshot?.error && (
        <div className="community-banner error">
          {t(`community.error.${snapshot.error.code}`, {
            defaultValue: t('community.error.unknown'),
          })}
          <span className="tiny"> {snapshot.error.detail}</span>
        </div>
      )}

      {loading ? (
        <div className="community-skeleton">
          <span className="spinner" />
          <span className="tiny">{t('community.loading')}</span>
        </div>
      ) : filtered.length === 0 ? (
        <div className="empty-state community-empty">
          <h2>{items.length === 0 ? t('community.emptyTitle') : t('community.noResults')}</h2>
          <p>{items.length === 0 ? t('community.emptyBody') : t('community.noResultsBody')}</p>
          {items.length === 0 && (
            <button type="button" className="button" onClick={actions.onRefresh}>
              {t('community.retry')}
            </button>
          )}
        </div>
      ) : (
        <>
          <div className="mod-grid community-grid" role="list">
            {visible.map((item) => (
              <CommunityCard key={item.folder} item={item} onOpen={setSelectedFolder} />
            ))}
          </div>
          {pageCount > 1 && (
            <div className="library-pager">
              <button
                type="button"
                className="button ghost"
                disabled={safePage === 0}
                onClick={() => setPage(safePage - 1)}
              >
                {t('community.pagePrev')}
              </button>
              <span className="pager-status">
                {t('community.pageStatus', { page: safePage + 1, pages: pageCount })}
              </span>
              <button
                type="button"
                className="button ghost"
                disabled={safePage >= pageCount - 1}
                onClick={() => setPage(safePage + 1)}
              >
                {t('community.pageNext')}
              </button>
            </div>
          )}
        </>
      )}

      {selected && (
        <DetailDrawer label={selected.name} onClose={() => setSelectedFolder(null)}>
          <CommunityDetail item={selected} job={job} busy={busy} actions={actions} />
        </DetailDrawer>
      )}
    </section>
  )
}
