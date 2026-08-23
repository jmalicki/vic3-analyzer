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
    <section className="desktop-catalog" aria-label="Desktop save catalog">
      <div className="desktop-catalog-status">
        <div>
          <p className="eyebrow">DESKTOP</p>
          <strong>
            {dashboard?.config.game_dir
              ? dashboard.config.game_dir
              : dashboard?.game_detected
                ? 'Game folder auto-detected'
                : 'No game folder yet'}
          </strong>
          <p className="desktop-catalog-hint">
            Saves and definitions come from disk — configure paths in Settings if auto-detect is
            wrong.
          </p>
        </div>
        <div className="desktop-catalog-actions">
          <button type="button" className="secondary" id="refresh-saves" onClick={() => void refresh()}>
            Refresh list
          </button>
        </div>
      </div>

      {status && (
        <p className="desktop-catalog-live" id="saves-status" role="status">
          {status}
        </p>
      )}
      {error && (
        <p className="desktop-catalog-live" role="alert">
          {error}
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
