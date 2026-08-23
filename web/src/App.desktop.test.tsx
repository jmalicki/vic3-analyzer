import { cleanup, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
import { invokeTauri } from './tauriApi'

vi.mock('./env', () => ({
  isTauri: () => true,
}))

vi.mock('./tauriApi', () => ({
  invokeTauri: vi.fn(),
  createTauriApi: () => ({
    classify_defs_path: () => 'skip',
    DefsBlobBuilder: class {},
    what_if_schema: () => '{}',
    prices_schema: () => '{}',
    build_defs_blob: async () => new Uint8Array(),
    defs_summary: async () => '{}',
    defs_icons: async () => '{}',
    parse_save: async () => '{}',
    load_analysis: async () => '{}',
    clear_analysis: async () => {},
    loaded_prices: async () => '{}',
    loaded_military: async () => '{}',
    loaded_constructions: async () => '{}',
    export_save: async () => new Uint8Array(),
    loaded_what_if: async () => '{}',
    loaded_apply_delta: async () => '{}',
    loaded_optimize_pms: async () => '{}',
    loaded_gaps: async () => '{}',
    loaded_plan: async () => '{}',
    loaded_alerts: async () => JSON.stringify({ alerts: [], limitations: [] }),
    loaded_production_methods: async () => '{}',
    prices: async () => '{}',
    what_if: async () => '{}',
    gaps: async () => '{}',
    plan: async () => '{}',
  }),
}))

const invoke = vi.mocked(invokeTauri)

describe('App (desktop / Tauri)', () => {
  beforeEach(() => {
    invoke.mockReset()
    invoke.mockImplementation(async (cmd) => {
      if (cmd === 'get_dashboard') {
        return {
          config: {
            game_dir: '/Victoria 3/game',
            defs_blob: null,
            save_dirs: [],
            tokens_path: null,
            auto_detect: true,
            config_path: '/cfg.toml',
          },
          game_detected: true,
          save_root_count: 1,
          save_count: 0,
          loaded_stub: null,
          detection_hints: [],
        }
      }
      if (cmd === 'list_saves') return []
      throw new Error(`unexpected ${cmd}`)
    })
  })

  afterEach(() => {
    cleanup()
  })

  it('hides the upload drop zone and shows the catalog', async () => {
    render(<App />)
    await waitFor(() => {
      expect(screen.getByLabelText(/Desktop save catalog/i)).toBeInTheDocument()
    })
    expect(screen.queryByLabelText(/Analysis files/i)).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Settings' })).toBeInTheDocument()
    expect(
      screen.getByText(/auto-detect game and saves/i),
    ).toBeInTheDocument()
  })
})
