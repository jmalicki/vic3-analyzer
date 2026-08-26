import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
import { invokeTauri } from './tauriApi'

vi.mock('./env', () => ({
  isTauri: () => true,
}))

vi.mock('./tauriApi', () => {
  const invokeTauri = vi.fn()
  return {
    invokeTauri,
    createTauriApi: () => ({
      classify_defs_path: () => 'skip',
      DefsBlobBuilder: class {},
      what_if_schema: () => '{}',
      prices_schema: () => '{}',
      build_defs_blob: async () => new Uint8Array(),
      defs_summary: async () => '{}',
      defs_icons: async () => invokeTauri('loaded_defs_icons'),
      parse_save: async () => '{}',
      load_analysis: async () => '{}',
      clear_analysis: async () => {},
      loaded_prices: async () => invokeTauri('loaded_prices'),
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
      prices: async () => invokeTauri('loaded_prices'),
      what_if: async () => '{}',
      gaps: async () => '{}',
      plan: async () => '{}',
    }),
  }
})

const invoke = vi.mocked(invokeTauri)

const FLAG_PNG = 'data:image/png;base64,FLAGBDG'
const IRON_ICON = 'data:image/png;base64,IRONICON'

const desktopPrices = {
  goods: [{ good_name: 'iron', good_label: 'Iron', base: 40, price: 43.5, buy: 120, sell: 100 }],
  countries: [
    { id: 10, tag: 'ALP', name: 'Alpacania', flag_data_url: 'data:image/png;base64,FLAGALP' },
    { id: 20, tag: 'BDG', name: 'Badgeria', flag_data_url: FLAG_PNG },
  ],
  states: [
    {
      id: 1,
      region_id: 'STATE_ALPACA',
      state_name: 'Alpaca',
      country_id: 10,
      market_id: 1,
    },
    {
      id: 3,
      region_id: 'STATE_BADGER',
      state_name: 'Badger',
      country_id: 20,
      market_id: 1,
    },
  ],
  state_pops: [],
  residual: 0,
  status: 'converged',
  limitations: [],
}

const desktopSummary = {
  tag: 'ALP',
  country_id: 10,
  market_id: 1,
  date: '1840.1.1',
  version: '1.9.0',
}

const saveStub = {
  name: 'alp.v3',
  kind: 'autosave',
  location: 'local',
  mtime: 1,
  in_game_date: '1840.1.1',
  country: 'ALP',
}

describe('App (desktop / Tauri)', () => {
  beforeEach(() => {
    window.location.hash = ''
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
          save_count: 1,
          loaded_stub: null,
          detection_hints: [],
        }
      }
      if (cmd === 'list_saves') return [saveStub]
      if (cmd === 'use_save') {
        return JSON.stringify({ summary: desktopSummary })
      }
      if (cmd === 'loaded_prices') return JSON.stringify(desktopPrices)
      if (cmd === 'loaded_defs_icons') {
        return JSON.stringify({ iron: IRON_ICON })
      }
      throw new Error(`unexpected ${cmd}`)
    })
  })

  afterEach(() => {
    cleanup()
  })

  it('hides upload, shows a compact save chip, and puts the catalog on Saves', async () => {
    const user = userEvent.setup()
    render(<App />)

    await waitFor(() => {
      expect(screen.getByLabelText(/Loaded save/i)).toBeInTheDocument()
    })
    expect(screen.queryByLabelText(/Analysis files/i)).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Choose save/i })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Saves' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Settings' })).toBeInTheDocument()

    // Default desktop landing is the Saves workspace.
    await waitFor(() => {
      expect(screen.getByLabelText(/Desktop save catalog/i)).toBeInTheDocument()
    })

    await user.click(screen.getByRole('button', { name: 'Prices' }))
    await waitFor(() => {
      expect(screen.queryByLabelText(/Desktop save catalog/i)).not.toBeInTheDocument()
    })
    expect(screen.getByLabelText(/Loaded save/i)).toBeInTheDocument()
  })

  it('loads goods icons and foreign state flags after binding a catalog save', async () => {
    const user = userEvent.setup()
    const { container } = render(<App />)

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Load' })).toBeInTheDocument()
    })
    await user.click(screen.getByRole('button', { name: 'Load' }))

    expect(await screen.findByText('Iron')).toBeInTheDocument()
    const goodIcon = container.querySelector('img.good-icon')
    expect(goodIcon).toHaveAttribute('src', IRON_ICON)

    await user.click(screen.getByRole('button', { name: 'States' }))
    expect(await screen.findByRole('link', { name: /Badger/ })).toBeInTheDocument()

    const flag = container.querySelector('img.country-flag')
    expect(flag).toHaveAttribute('src', FLAG_PNG)
    // Player-owned Alpaca stays foreign-only (no flag in the list).
    expect(screen.getByRole('link', { name: /^Alpaca$/ })).toBeInTheDocument()
    expect(container.querySelectorAll('img.country-flag')).toHaveLength(1)
  })
})
