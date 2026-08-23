import { useCallback, useEffect, useState } from 'react'
import type { ConfigDto } from './SettingsPane'
import { invokeTauri } from './tauriApi'

export interface SaveStub {
  name: string
  kind: string
  location: string
  mtime: number
  in_game_date?: string | null
  country?: string | null
}

export interface DashboardDto {
  config: ConfigDto
  game_detected: boolean
  save_root_count: number
  save_count: number
  loaded_stub: string | null
  detection_hints: string[]
}

interface Props {
  /** Currently bound save stub (filename). */
  loadedName?: string
  onUseSave: (stub: SaveStub) => Promise<void>
  /** Bump to force a catalog refresh (e.g. after Settings save). */
  refreshKey?: number
}

function formatMtime(mtime: number): string {
  if (!Number.isFinite(mtime) || mtime <= 0) return '—'
  try {
    return new Date(mtime * 1000).toLocaleString()
  } catch {
    return '—'
  }
}

/** Full-page save picker (desktop `#/saves`). */
export function DesktopCatalog({ loadedName, onUseSave, refreshKey = 0 }: Props) {
  const [dashboard, setDashboard] = useState<DashboardDto>()
  const [saves, setSaves] = useState<SaveStub[]>([])
  const [status, setStatus] = useState<string>()
  const [error, setError] = useState<string>()
  const [busy, setBusy] = useState(false)
  const [loadingName, setLoadingName] = useState<string>()

  const refresh = useCallback(async () => {
    setError(undefined)
    try {
      const [dash, rows] = await Promise.all([
        invokeTauri<DashboardDto>('get_dashboard'),
        invokeTauri<SaveStub[]>('list_saves'),
      ])
      setDashboard(dash)
      setSaves(rows)
      setStatus(
        dash.game_detected
          ? `Game folder ready · ${dash.save_count} save${dash.save_count === 1 ? '' : 's'}`
          : 'Game folder not detected — open Settings to set the path or reset auto-detect',
      )
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh, refreshKey])

  const load = async (stub: SaveStub) => {
    setBusy(true)
    setLoadingName(stub.name)
    setError(undefined)
    try {
      await onUseSave(stub)
      setStatus(`Loaded ${stub.name}`)
      await refresh()
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setBusy(false)
      setLoadingName(undefined)
    }
  }

  return (
    <section className="workspace-page" aria-label="Desktop save catalog">
      <div className="tool-heading">
        <div>
          <p className="eyebrow">DESKTOP</p>
          <h2 id="saves-heading">Saves</h2>
          <p>
            Pick a cataloged save from disk. Paths stay in Rust — configure the game folder in
            Settings if auto-detect is wrong.
          </p>
        </div>
        <button type="button" className="secondary" id="refresh-saves" onClick={() => void refresh()}>
          Refresh list
        </button>
      </div>

      <p className="desktop-catalog-live" id="saves-status" role="status">
        {dashboard?.config.game_dir
          ? dashboard.config.game_dir
          : dashboard?.game_detected
            ? 'Game folder auto-detected'
            : 'No game folder yet'}
        {status ? ` · ${status}` : ''}
      </p>
      {error && (
        <p className="desktop-catalog-live" role="alert">
          {error}
        </p>
      )}
      {busy && (
        <p className="desktop-catalog-live" role="status">
          Loading {loadingName}… the window should stay responsive while analysis runs in the
          background.
        </p>
      )}

      {saves.length === 0 ? (
        <p className="desktop-catalog-empty">
          No saves in the catalog yet. Check save folders in Settings, then refresh.
        </p>
      ) : (
        <div className="desktop-catalog-table-wrap">
          <table className="desktop-catalog-table">
            <thead>
              <tr>
                <th>Name</th>
                <th>Kind</th>
                <th>Location</th>
                <th>Date</th>
                <th>mtime</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {saves.map((stub) => {
                const key = `${stub.location}:${stub.name}`
                const selected = loadedName === stub.name
                return (
                  <tr key={key} data-selected={selected || undefined}>
                    <td>{stub.name}</td>
                    <td>{stub.kind}</td>
                    <td>{stub.location}</td>
                    <td>{stub.in_game_date ?? '—'}</td>
                    <td>{formatMtime(stub.mtime)}</td>
                    <td>
                      <button
                        type="button"
                        disabled={busy}
                        aria-current={selected ? 'true' : undefined}
                        onClick={() => void load(stub)}
                      >
                        {loadingName === stub.name ? 'Loading…' : selected ? 'Reload' : 'Load'}
                      </button>
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
      )}
    </section>
  )
}

interface ChipProps {
  loadedName?: string
  summaryTag?: string
  summaryDate?: string
  busy?: boolean
  onOpenSaves: () => void
}

/** Compact loaded-save chip for the desktop chrome (not the full catalog). */
export function DesktopSaveChip({
  loadedName,
  summaryTag,
  summaryDate,
  busy,
  onOpenSaves,
}: ChipProps) {
  const detail =
    loadedName && (summaryTag || summaryDate)
      ? [summaryTag, summaryDate].filter(Boolean).join(' · ')
      : undefined

  return (
    <div className="desktop-save-chip" role="status" aria-label="Loaded save">
      <div className="desktop-save-chip-text">
        <span className="hud-label">Save</span>
        <strong>{busy ? 'Loading…' : loadedName ?? 'None loaded'}</strong>
        {detail && !busy ? <span className="desktop-save-chip-detail">{detail}</span> : null}
      </div>
      <button type="button" className="secondary" onClick={onOpenSaves}>
        {loadedName ? 'Change save…' : 'Choose save…'}
      </button>
    </div>
  )
}
