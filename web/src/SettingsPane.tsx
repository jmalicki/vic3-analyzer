import { useEffect, useState, type FormEvent } from 'react'
import { invokeTauri } from './tauriApi'

export interface ConfigDto {
  game_dir: string | null
  defs_blob: string | null
  save_dirs: string[]
  tokens_path: string | null
  auto_detect: boolean
  config_path: string
}

interface Props {
  /** Called after a successful save/reset so the catalog can refresh. */
  onConfigChange?: (config: ConfigDto) => void
}

export function SettingsPane({ onConfigChange }: Props) {
  const [gameDir, setGameDir] = useState('')
  const [defsBlob, setDefsBlob] = useState('')
  const [saveDirs, setSaveDirs] = useState('')
  const [tokensPath, setTokensPath] = useState('')
  const [autoDetect, setAutoDetect] = useState(true)
  const [configPath, setConfigPath] = useState('')
  const [status, setStatus] = useState<string>()
  const [error, setError] = useState<string>()
  const [busy, setBusy] = useState(false)

  const applyConfig = (cfg: ConfigDto) => {
    setGameDir(cfg.game_dir ?? '')
    setDefsBlob(cfg.defs_blob ?? '')
    setSaveDirs((cfg.save_dirs ?? []).join('\n'))
    setTokensPath(cfg.tokens_path ?? '')
    setAutoDetect(cfg.auto_detect)
    setConfigPath(cfg.config_path)
  }

  useEffect(() => {
    let cancelled = false
    void invokeTauri<ConfigDto>('get_config')
      .then((cfg) => {
        if (!cancelled) applyConfig(cfg)
      })
      .catch((reason: unknown) => {
        if (!cancelled) setError(reason instanceof Error ? reason.message : String(reason))
      })
    return () => {
      cancelled = true
    }
  }, [])

  const toDto = (): ConfigDto => ({
    game_dir: gameDir.trim() || null,
    defs_blob: defsBlob.trim() || null,
    save_dirs: saveDirs
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean),
    tokens_path: tokensPath.trim() || null,
    auto_detect: autoDetect,
    config_path: configPath,
  })

  const save = async (event?: FormEvent) => {
    event?.preventDefault()
    setBusy(true)
    setError(undefined)
    setStatus(undefined)
    try {
      const cfg = await invokeTauri<ConfigDto>('save_config', { config: toDto() })
      applyConfig(cfg)
      setStatus('Saved')
      onConfigChange?.(cfg)
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setBusy(false)
    }
  }

  const reset = async () => {
    setBusy(true)
    setError(undefined)
    setStatus(undefined)
    try {
      const cfg = await invokeTauri<ConfigDto>('reset_config')
      applyConfig(cfg)
      setStatus('Reset to auto-detect')
      onConfigChange?.(cfg)
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="workspace-page" aria-labelledby="settings-heading" id="view-settings">
      <div className="tool-heading">
        <div>
          <p className="eyebrow">DESKTOP</p>
          <h2 id="settings-heading">Settings</h2>
          <p>
            Shared with <code>vic3-analyzer mcp</code> via app-data <code>config.toml</code>. The
            desktop app reads the game folder and save directories from here — no browser upload.
          </p>
        </div>
      </div>

      <form className="settings-form" onSubmit={(event) => void save(event)}>
        <label>
          Game folder (<code>…/Victoria 3/game</code>)
          <input
            id="cfg-game"
            type="text"
            spellCheck={false}
            value={gameDir}
            onChange={(event) => setGameDir(event.target.value)}
            placeholder="Leave blank to auto-detect"
          />
        </label>
        <label>
          Defs postcard (optional — skips live install)
          <input
            id="cfg-defs"
            type="text"
            spellCheck={false}
            value={defsBlob}
            onChange={(event) => setDefsBlob(event.target.value)}
          />
        </label>
        <label>
          Save folders (one path per line)
          <textarea
            id="cfg-saves"
            spellCheck={false}
            rows={4}
            value={saveDirs}
            onChange={(event) => setSaveDirs(event.target.value)}
            placeholder="Leave blank to auto-detect"
          />
        </label>
        <label>
          Token map (optional, ironman/binary)
          <input
            id="cfg-tokens"
            type="text"
            spellCheck={false}
            value={tokensPath}
            onChange={(event) => setTokensPath(event.target.value)}
          />
        </label>
        <label className="settings-check" htmlFor="cfg-auto">
          <input
            id="cfg-auto"
            type="checkbox"
            checked={autoDetect}
            onChange={(event) => setAutoDetect(event.target.checked)}
          />
          Auto-detect when paths are missing
        </label>
        {configPath && (
          <p className="settings-config-path" id="cfg-path">
            {configPath}
          </p>
        )}
        <div className="settings-actions">
          <button type="submit" id="save-settings" disabled={busy}>
            Save config
          </button>
          <button
            type="button"
            className="secondary"
            id="reset-settings"
            disabled={busy}
            onClick={() => void reset()}
          >
            Reset to auto-detect
          </button>
        </div>
      </form>

      {status && (
        <p className="settings-status" id="settings-status" role="status">
          {status}
        </p>
      )}
      {error && (
        <p className="settings-status" role="alert">
          {error}
        </p>
      )}
    </section>
  )
}
