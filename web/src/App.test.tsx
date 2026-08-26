import { cleanup, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
import { loadWasmApi } from './wasmClient'
import { clearAnalyses, listAnalyses, saveAnalysis } from './archive'
import { clearStoredDefs, storeDefs } from './defsStore'
import { clearStoredSave, loadStoredSave, storeSave, storeSaveAnalysis } from './saveStore'
import type { AnalysisRecord, PricesResult } from './types'
import type { WasmApi } from './wasm'

vi.mock('./wasmClient', () => ({
  loadWasmApi: vi.fn(),
}))

/** Test-only blobs; silly names so they never look like a real install export. */
const result = JSON.stringify({
  goods: [{ good_name: 'iron', base: 40, price: 43.5, buy: 120, sell: 100 }],
  states: [
    {
      id: 1,
      region_id: 'STATE_ILE_DE_FRANCE',
      state_name: 'Ile-de-France',
      country_id: 16777216,
      market_id: 1,
    },
  ],
  state_pops: [{ state_id: 1, workforce: 8000, dependents: 4000, wealth: 12 }],
  buildings: [
    {
      id: 9,
      state_id: 1,
      type_id: 'building_rye_farm',
      level: 4,
      staffing: 3.2,
      production_method_ids: ['pm_simple_farming'],
      inputs: [],
      outputs: [{ good_name: 'grain', quantity: 10, value: 200 }],
      revenue: 200,
      cost: 50,
      profit: 150,
      short_inputs: [],
    },
  ],
  building_types: [{ id: 'building_rye_farm', name: 'Rye Farms' }],
  residual: 0.00001,
  status: 'converged',
  limitations: ['Employment and production methods stay frozen.'],
})

const gapsResult = JSON.stringify({
  satisfied: false,
  gaps: [
    { HasTech: 'nitroglycerin' },
    { GoodPrice: { good: 'ammunition', rel: 'Le', value: 40 } },
    'Solvent',
  ],
  limitations: ['Goal gaps use prices from a frozen-world solve.'],
})

const planResult = JSON.stringify({
  day_cost: 365,
  actions: [
    { day: 0, action: { QueueTech: { tech: 'nitroglycerin' } } },
    {
      day: 365,
      action: {
        WaitForEvent: {
          event: { TechCompleted: { tech: 'nitroglycerin' } },
          days: 365,
        },
      },
    },
  ],
  residual: 0.00001,
  limitations: ['Research duration is fixed by the compact simulator.'],
})

const saveSummary = {
  tag: 'FRA',
  country_id: 16777216,
  market_id: 1,
  date: '1840.2.3',
  version: '1.9.0',
  buildings: ['building_rye_farm', 'building_steel_mills'],
}

const schema = JSON.stringify({
  title: 'WhatIfOpts',
  type: 'object',
  required: ['building', 'extra_levels'],
  properties: {
    building: { type: 'string', description: 'Building type id.' },
    extra_levels: { type: 'integer', minimum: 0 },
  },
})

function mockApi(): WasmApi {
  return {
    classify_defs_path: vi.fn(() => 'read' as const),
    DefsBlobBuilder: class {
      addBatch() {}
      neededGfxNames() {
        return '[]'
      }
      finish() {
        return new Uint8Array([7, 8, 9])
      }
    },
    build_defs_blob: vi.fn(() => new Uint8Array([7, 8, 9])),
    defs_icons: vi.fn(() => JSON.stringify({ iron: 'data:image/png;base64,IRONICON' })),
    defs_summary: vi.fn(() =>
      JSON.stringify({
        blob_version: 6,
        goods: 3,
        labels: 3,
        icons: 1,
        production_methods: 5,
        pop_needs: 2,
        buy_packages: 1,
        price_range: 0.75,
      }),
    ),
    parse_save: vi.fn(() => JSON.stringify(saveSummary)),
    load_analysis: vi.fn(() =>
      JSON.stringify({ summary: saveSummary, prices: JSON.parse(result) }),
    ),
    clear_analysis: vi.fn(),
    loaded_prices: vi.fn(() => result),
    loaded_military: vi.fn(() =>
      JSON.stringify({
        armies: [
          {
            id: 1,
            name: 'Armée du Nord',
            type: 'army',
            organization: 85,
            current_manpower: 12000,
            units: [{ id: 11, name: '1st Infantry', type: 'line_infantry', manpower: 1000 }],
          },
        ],
        navies: [
          {
            id: 2,
            name: 'Atlantic Fleet',
            type: 'navy',
            current_manpower: 4000,
            units: [{ id: 21, name: 'HMS Vic', type: 'man_o_war' }],
          },
        ],
        mobilization: [{ id: 3, name: 'General Mobilization', type: 'general' }],
        limitations: [],
      }),
    ),
    loaded_constructions: vi.fn(() =>
      JSON.stringify({
        private: [
          {
            id: 10,
            queue: 'private',
            building: 'building_logging_camp',
            building_name: 'Logging Camp',
            remaining: 5,
          },
        ],
        government: [
          {
            id: 1,
            queue: 'government',
            building: 'building_construction_sector',
            building_name: 'Construction Sector',
            remaining: 40,
          },
        ],
      }),
    ),
    export_save: vi.fn(() => new Uint8Array([1, 2, 3])),
    loaded_what_if: vi.fn(() => result),
    loaded_apply_delta: vi.fn(() => result),
    loaded_optimize_pms: vi.fn(() =>
      JSON.stringify({
        axis: 'productivity',
        changes: [],
        delta: { income: 0, productivity: 0, sol: 0, residual: 0 },
        limitations: [],
        world_delta: {},
      }),
    ),
    loaded_gaps: vi.fn(() => gapsResult),
    loaded_plan: vi.fn(() => planResult),
    loaded_alerts: vi.fn(() =>
      JSON.stringify({
        alerts: [],
        limitations: ['Apply is disabled until the apply track.'],
      }),
    ),
    loaded_production_methods: vi.fn(() =>
      JSON.stringify([
        { id: 'pm_simple_farming', inputs: [], outputs: [{ good: 'grain', qty: 20 }] },
      ]),
    ),
    prices: vi.fn(() => result),
    what_if: vi.fn(() => result),
    gaps: vi.fn(() => gapsResult),
    plan: vi.fn(() => planResult),
    what_if_schema: vi.fn(() => schema),
    prices_schema: vi.fn(() => '{}'),
  }
}

function mockBundledDefs(ok = true) {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (!url.includes('defs.postcard')) {
        return new Response(null, { status: 404 })
      }
      if (!ok) return new Response(null, { status: 404 })
      return new Response(new Uint8Array([1, 2, 3]), {
        status: 200,
        headers: { 'Content-Type': 'application/octet-stream' },
      })
    }),
  )
}

async function selectSave(user: ReturnType<typeof userEvent.setup>) {
  await user.upload(screen.getByLabelText('Save file'), new File(['save'], 'campaign.v3'))
  await screen.findByText('FRA')
  await screen.findByText(/Using the local development demo blob/)
}

async function buildDefinitions(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole('button', { name: 'Build definitions from game files…' }))
  const file = new File(['grain = { cost = 20 }'], 'goods.txt')
  Object.defineProperty(file, 'webkitRelativePath', {
    value: 'Victoria 3/game/common/goods/goods.txt',
  })
  await user.upload(screen.getByLabelText('Victoria 3 definitions folder'), file)
  await screen.findByText(/Built defs\.postcard/)
}

describe('prices UI', () => {
  beforeEach(async () => {
    window.location.hash = ''
    await clearAnalyses()
    await clearStoredDefs()
    await clearStoredSave()
    mockBundledDefs()
  })
  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
  })

  it('renders mocked wasm goods and limitations after loading a save', async () => {
    const user = userEvent.setup()
    const api = mockApi()
    render(<App wasmApi={api} />)
    await selectSave(user)

    expect(await screen.findByText('Iron')).toBeInTheDocument()
    expect(screen.getByText('43.50')).toBeInTheDocument()
    expect(screen.getAllByRole('heading', { name: 'Goods prices' })).toHaveLength(1)
    expect(screen.getByRole('heading', { name: 'Goods prices' }).closest('.workspace-page')).toHaveTextContent(
      'Iron',
    )
    expect(screen.getByRole('link', { name: 'Method and limitations' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Analyze prices' })).not.toBeInTheDocument()
    expect(api.load_analysis).toHaveBeenCalled()
  })

  it('shows the game icon for a priced good', async () => {
    const user = userEvent.setup()
    const { container } = render(<App wasmApi={mockApi()} />)
    await selectSave(user)
    await screen.findByText('Iron')

    const icon = container.querySelector('img.good-icon')
    expect(icon).toHaveAttribute('src', 'data:image/png;base64,IRONICON')
  })

  it('reports what the active definitions blob contains', async () => {
    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)
    await selectSave(user)

    expect(
      await screen.findByText(/3 goods, 3 names, 1 icons, 5 production methods/),
    ).toBeInTheDocument()
    expect(
      screen.getByText(/The local development demo blob defines only a few fixture goods/),
    ).toBeInTheDocument()
  })

  it('drops the thin-definitions warning once a full blob is loaded', async () => {
    const user = userEvent.setup()
    const api = mockApi()
    api.defs_summary = vi.fn(() =>
      JSON.stringify({
        blob_version: 6,
        goods: 53,
        labels: 53,
        icons: 53,
        production_methods: 412,
        pop_needs: 7,
        buy_packages: 5,
        price_range: 0.75,
      }),
    )
    render(<App wasmApi={api} />)
    await selectSave(user)

    expect(
      await screen.findByText(/53 goods, 53 names, 53 icons, 412 production methods/),
    ).toBeInTheDocument()
    expect(
      screen.queryByText(/The local development demo blob defines only a few fixture goods/),
    ).not.toBeInTheDocument()
  })

  it('rebuilds prices when the definitions change', async () => {
    const user = userEvent.setup()
    const api = mockApi()
    render(<App wasmApi={api} />)
    await selectSave(user)
    expect(await screen.findByText('Iron')).toBeInTheDocument()

    await buildDefinitions(user)

    await waitFor(() => expect(api.load_analysis).toHaveBeenCalledTimes(2))
    expect(screen.getByText('Iron')).toBeInTheDocument()
  })

  it('flags a user blob that is missing common/goods', async () => {
    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)
    await selectSave(user)

    await buildDefinitions(user)

    expect(
      await screen.findByText(/defs\.postcard only defines 3 goods/),
    ).toBeInTheDocument()
  })

  it('builds and submits the what-if form from the wasm schema', async () => {
    const user = userEvent.setup()
    const api = mockApi()
    render(<App wasmApi={api} />)
    await selectSave(user)
    await user.click(screen.getByRole('button', { name: 'What-if' }))

    await user.selectOptions(screen.getByLabelText('Building'), 'building_steel_mills')
    await user.clear(screen.getByLabelText('Extra Levels'))
    await user.type(screen.getByLabelText('Extra Levels'), '5')
    await user.click(screen.getByRole('button', { name: 'Run what-if' }))

    await waitFor(() =>
      expect(api.loaded_what_if).toHaveBeenCalledWith(
        JSON.stringify({ building: 'building_steel_mills', extra_levels: 5 }),
      ),
    )
    expect((await listAnalyses())[0].kind).toBe('what_if')
  })

  it('renders mocked gap atoms, satisfaction, and limitations', async () => {
    const user = userEvent.setup()
    const api = mockApi()
    render(<App wasmApi={api} />)
    await selectSave(user)
    await user.click(screen.getByRole('button', { name: 'Goal gaps' }))

    await user.click(screen.getByRole('button', { name: 'Check readiness' }))

    expect(await screen.findByText('Satisfied: No')).toBeInTheDocument()
    expect(screen.getByText('{"HasTech":"nitroglycerin"}')).toBeInTheDocument()
    expect(
      screen.getByText('{"GoodPrice":{"good":"ammunition","rel":"Le","value":40}}'),
    ).toBeInTheDocument()
    expect(screen.getByText('Solvent')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Method and limitations' })).toBeInTheDocument()
    expect(api.loaded_gaps).toHaveBeenCalledWith('research(tech=nitroglycerin)')
    await waitFor(async () => expect((await listAnalyses())[0].kind).toBe('gaps'))
  })

  it('renders a mocked plan timeline and archives its label', async () => {
    const user = userEvent.setup()
    const api = mockApi()
    render(<App wasmApi={api} />)
    await selectSave(user)
    await user.click(screen.getByRole('button', { name: 'Timeline' }))

    await user.type(screen.getByLabelText('Plan label'), 'rush')
    await user.click(screen.getByRole('button', { name: 'Build timeline' }))

    expect(await screen.findByText('365 total days')).toBeInTheDocument()
    expect(screen.getByText('Queue technology: nitroglycerin')).toBeInTheDocument()
    expect(screen.getByText('Wait 365 days for nitroglycerin')).toBeInTheDocument()
    await waitFor(() => expect(api.loaded_plan).toHaveBeenCalled())
    const records = await listAnalyses()
    expect(records[0]).toMatchObject({ kind: 'plan', label: 'rush' })
    expect(records[0].result).toMatchObject({ day_cost: 365 })
  })

  it('applies a default plan goal and archive label', async () => {
    const user = userEvent.setup()
    const api = mockApi()
    render(<App wasmApi={api} />)
    await selectSave(user)
    await user.click(screen.getByRole('button', { name: 'Timeline' }))

    await user.selectOptions(screen.getByLabelText('Plan default plan'), 'economic-growth')

    expect(screen.getByLabelText('Plan label')).toHaveValue('GDP 100 million')
    expect(screen.getByText('Goal: gdp >= 100000000')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Build timeline' }))

    await waitFor(() =>
      expect(api.loaded_plan).toHaveBeenCalledWith(
        JSON.stringify({
          goal: 'gdp >= 100000000',
          max_days: 3650,
          label: 'GDP 100 million',
        }),
      ),
    )
  })

  it('offers fiscal and standard-of-living readiness defaults', async () => {
    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)
    await selectSave(user)
    await user.click(screen.getByRole('button', { name: 'Goal gaps' }))

    const picker = screen.getByLabelText('Gaps default plan')
    expect(picker).toHaveTextContent('Increase weekly income')
    expect(picker).toHaveTextContent('Raise standard of living')
    await user.selectOptions(picker, 'standard-of-living')
    expect(screen.getByText('Goal: population_weighted_wealth >= 20')).toBeInTheDocument()
  })

  it('marks war and income/SoL presets as gaps-only on the timeline picker', async () => {
    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)
    await selectSave(user)
    await user.click(screen.getByRole('button', { name: 'Timeline' }))

    const picker = screen.getByLabelText('Plan default plan')
    expect(picker).toHaveTextContent('Prepare for war (gaps only)')
    expect(picker).toHaveTextContent('Build a good-sized military')
    expect(picker).toHaveTextContent('Increase weekly income (gaps only)')
    expect(picker).toHaveTextContent('Raise standard of living (gaps only)')
    expect(picker).toHaveTextContent('Avoid default')
    expect(picker).not.toHaveTextContent('Avoid default (gaps only)')
    expect(picker).toHaveTextContent('Grow the economy')
    expect(picker.querySelector('option[value="war-readiness"]')).toBeDisabled()
    expect(picker.querySelector('option[value="military-size"]')).not.toBeDisabled()
    expect(picker.querySelector('option[value="avoid-default"]')).not.toBeDisabled()
    expect(picker.querySelector('option[value="economic-growth"]')).not.toBeDisabled()
  })

  it('compares two mocked archived plan records', async () => {
    const planRecord = (id: string, label: string, dayCost: number): AnalysisRecord => ({
      id,
      created_at: `2026-08-15T12:0${id === 'left' ? '0' : '1'}:00.000Z`,
      label,
      kind: 'plan',
      fingerprint: 'same-save',
      date: '1840.2.3',
      country: 'FRA',
      opts: { goal: 'research(tech=nitroglycerin)' },
      result: {
        day_cost: dayCost,
        actions: [],
        residual: 0,
        limitations: [],
      },
      limitations: [],
    })
    await saveAnalysis(planRecord('left', 'rush', 365))
    await saveAnalysis(planRecord('right', 'steady', 480))

    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)
    await user.click(screen.getByRole('button', { name: 'Archive' }))
    await user.click(await screen.findByLabelText('Compare rush'))
    await user.click(screen.getByLabelText('Compare steady'))

    expect(await screen.findByRole('heading', { name: 'Archive comparison' })).toBeInTheDocument()
    expect(screen.getByText('Alternative plans')).toBeInTheDocument()
    expect(screen.getByText('+115')).toBeInTheDocument()
  })

  it('explains token maps and definitions through accessible help', async () => {
    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)

    await user.click(screen.getByRole('button', { name: 'About token maps' }))
    const tokenHelp = await screen.findByRole('region', { name: 'About token maps' })
    expect(tokenHelp).toHaveTextContent('0x1234 field_name')
    expect(tokenHelp).toHaveTextContent('Most players do not need one.')
    expect(tokenHelp).toHaveTextContent('"save_file_format": "zip_text_all"')
    expect(tokenHelp).toHaveTextContent('There is no official download')
    expect(tokenHelp).toHaveTextContent('extracted from your own game build')
    expect(screen.getByRole('link', { name: 'Victoria 3 wiki' })).toHaveAttribute(
      'href',
      'https://vic3.paradoxwikis.com/Save-game_editing',
    )

    await user.click(screen.getByRole('button', { name: 'About definitions' }))
    const defsHelp = await screen.findByRole('region', { name: 'About definitions' })
    expect(defsHelp).toHaveTextContent('postcard-encoded snapshot of goods')
  })

  it('uses definitions built locally instead of the dev-only demo blob', async () => {
    const user = userEvent.setup()
    const api = mockApi()
    api.DefsBlobBuilder = class {
      addBatch() {}
      neededGfxNames() {
        return '[]'
      }
      finish() {
        return new TextEncoder().encode('MOCKY-NOT-A-REAL-BLOB')
      }
    }
    render(<App wasmApi={api} />)
    await selectSave(user)

    await buildDefinitions(user)
    expect(await screen.findByText(/Using your file: defs\.postcard/)).toBeInTheDocument()

    await waitFor(() => expect(api.load_analysis).toHaveBeenCalled())
    const defsArg = vi.mocked(api.load_analysis).mock.calls.at(-1)?.[2]
    // Exact builder output proves the retained world used the locally built definitions.
    expect(new TextDecoder().decode(defsArg)).toBe('MOCKY-NOT-A-REAL-BLOB')
  })

  it('asks for definitions when no demo blob is served', async () => {
    mockBundledDefs(false)
    render(<App wasmApi={mockApi()} />)
    expect(await screen.findByText(/No definitions loaded/)).toBeInTheDocument()
    // Outside public/, so a production build has nothing to serve here.
    expect(vi.mocked(fetch).mock.calls.some(([url]) => String(url).includes('fixtures/defs.postcard'))).toBe(true)
  })

  it('keeps chosen definitions across a reload and can forget them', async () => {
    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)
    await selectSave(user)
    await buildDefinitions(user)
    await screen.findByText(/Using your file: defs\.postcard/)

    cleanup()
    render(<App wasmApi={mockApi()} />)
    expect(
      await screen.findByText(
        /Using your file: defs\.postcard.*kept from a previous visit/,
      ),
    ).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Forget these definitions' }))
    await screen.findByText(/Using the local development demo blob/)

    cleanup()
    render(<App wasmApi={mockApi()} />)
    expect(await screen.findByText(/Using the local development demo blob/)).toBeInTheDocument()
  })

  it('keeps the last save across a reload and can forget it', async () => {
    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)
    await selectSave(user)
    expect(screen.getByText('campaign.v3')).toBeInTheDocument()
    await waitFor(async () => {
      expect((await loadStoredSave())?.save.name).toBe('campaign.v3')
    })

    cleanup()
    render(<App wasmApi={mockApi()} />)
    expect(
      await screen.findByText(/campaign\.v3 \(kept from a previous visit\)/),
    ).toBeInTheDocument()
    expect(await screen.findByText('FRA')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Forget this save' }))
    await waitFor(() => expect(screen.queryByText(/campaign\.v3/)).not.toBeInTheDocument())

    cleanup()
    render(<App wasmApi={mockApi()} />)
    await screen.findByText(/Using the local development demo blob/)
    expect(screen.queryByText(/campaign\.v3/)).not.toBeInTheDocument()
  })

  it('shows cached prices immediately on reload without waiting for wasm', async () => {
    await storeSave(new File(['save'], 'campaign.v3'))
    await storeSaveAnalysis(saveSummary, JSON.parse(result) as PricesResult)
    const cached = await loadStoredSave()
    expect(cached?.summary?.tag).toBe('FRA')
    expect(cached?.prices?.goods[0]?.good_name).toBe('iron')
    const api = mockApi()
    api.load_analysis = vi.fn(() => new Promise<string>(() => {}))
    api.loaded_alerts = vi.fn(() => {
      throw new Error('no analysis is loaded')
    })
    render(<App wasmApi={api} />)

    expect(
      await screen.findByText(/campaign\.v3 \(kept from a previous visit\)/),
    ).toBeInTheDocument()
    expect(await screen.findByText('Iron')).toBeInTheDocument()
    expect(await screen.findByText('FRA')).toBeInTheDocument()
    expect(screen.getByText(/Showing the last analysis instantly/)).toBeInTheDocument()
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    expect(api.loaded_alerts).not.toHaveBeenCalled()
  })

  it('clears a stored blob when Rust rejects its format version', async () => {
    await storeDefs(new File(['OLD-V1-DEFS'], 'ancient-v1.postcard'))
    const api = mockApi()
    api.defs_summary = vi.fn((defs) => {
      if (new TextDecoder().decode(defs) === 'OLD-V1-DEFS') {
        throw new Error('defs blob version 1 is not supported (expected 6)')
      }
      return JSON.stringify({
        blob_version: 6,
        goods: 3,
        labels: 3,
        icons: 0,
        production_methods: 5,
        pop_needs: 2,
        buy_packages: 1,
        price_range: 0.75,
      })
    })

    render(<App wasmApi={api} />)

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'defs blob version 1 is not supported (expected 6). Rebuild definitions from your Victoria 3 game folder for this app version.',
    )
    await waitFor(() =>
      expect(screen.queryByText(/ancient-v1\.postcard/)).not.toBeInTheDocument(),
    )
  })

  it('uses the remembered Chromium picker when available', async () => {
    const picked = new File(['ironman'], 'ironman.v3')
    const showOpenFilePicker = vi.fn(async () => [{ getFile: async () => picked }])
    vi.stubGlobal('showOpenFilePicker', showOpenFilePicker)

    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)
    await screen.findByText(/Using the local development demo blob/)
    await user.click(screen.getByRole('button', { name: 'Choose save' }))

    expect(showOpenFilePicker).toHaveBeenCalledWith(
      expect.objectContaining({
        id: 'vic3-analyzer-save',
        startIn: 'documents',
      }),
    )
    expect(await screen.findByText('ironman.v3')).toBeInTheDocument()
  })

  it('ignores remembered-picker cancellation and keeps the file-input fallback', async () => {
    const showOpenFilePicker = vi.fn(async () => {
      throw new DOMException('The user aborted a request.', 'AbortError')
    })
    vi.stubGlobal('showOpenFilePicker', showOpenFilePicker)

    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)
    await user.click(screen.getByRole('button', { name: 'Choose save' }))
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()

    vi.unstubAllGlobals()
    mockBundledDefs()
    await user.upload(screen.getByLabelText('Save file'), new File(['save'], 'fallback.v3'))
    expect(await screen.findByText('fallback.v3')).toBeInTheDocument()
  })

  it('shows a platform save-path hint under the picker', async () => {
    render(<App wasmApi={mockApi()} />)
    expect(screen.getByText('Usual local folder')).toBeInTheDocument()
    expect(document.querySelector('.path-hint-path')?.textContent).toMatch(
      /Paradox Interactive[/\\]Victoria 3[/\\]save games/,
    )
  })

  it('opens the definitions builder in a modal dialog', async () => {
    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: /Build definitions from game files/ }))
    const dialog = await screen.findByRole('dialog', { name: 'Build definitions from game files' })
    expect(dialog).toHaveTextContent('Usual Steam folder')

    await user.click(screen.getByRole('button', { name: 'Close' }))
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  it('reports a failed engine load and keeps Choose game folder disabled', async () => {
    vi.mocked(loadWasmApi).mockRejectedValueOnce(new Error("Failed to construct 'URL'"))
    const user = userEvent.setup()
    render(<App />)

    expect(await screen.findByRole('alert')).toHaveTextContent(
      /Could not load the analysis engine: could not build a URL.*Failed to construct/,
    )

    await user.click(screen.getByRole('button', { name: /Build definitions from game files/ }))
    expect(screen.getByRole('button', { name: 'Choose game folder' })).toBeDisabled()
    expect(screen.getByText('Waiting for the analysis engine…')).toBeInTheDocument()
  })

  it('greys out analysis tools and names every missing input', async () => {
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockRejectedValue(new Error('no defs'))
    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)

    expect(
      await screen.findByText(/Analysis needs a \.v3 save and game definitions/),
    ).toBeInTheDocument()
    await waitFor(() =>
      expect(document.querySelector('.workspace-page')).toHaveClass('needs-defs'),
    )
    expect(screen.queryByRole('button', { name: 'Analyze prices' })).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'States' }))
    expect(screen.getByRole('heading', { name: 'States' })).toBeInTheDocument()
    expect(document.querySelector('.workspace-page')).toHaveClass('needs-defs')
    fetchMock.mockRestore()
  })

  it('drops the lock once a save and definitions are present', async () => {
    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)
    await selectSave(user)

    await waitFor(() =>
      expect(document.querySelector('.workspace-page')).not.toHaveClass('needs-defs'),
    )
    expect(screen.queryByText(/Analysis needs/)).not.toBeInTheDocument()
  })

  it('shows a campaign HUD and workbench panes after loading a save', async () => {
    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)
    await selectSave(user)

    const hud = screen.getByRole('region', { name: 'Campaign summary' })
    expect(hud).toHaveTextContent('FRA')
    expect(hud).toHaveTextContent('1840.2.3')
    expect(hud).toHaveTextContent('Victoria 3 1.9.0')
    expect(hud).toHaveTextContent('GDP')
    expect(screen.getByText('—', { selector: '.save-summary strong' })).toBeInTheDocument()
    await waitFor(() => expect(hud).toHaveTextContent('SoL'))
    expect(hud).toHaveTextContent('12.0')
    await waitFor(() => expect(hud).toHaveTextContent(/Alerts\s*0/))

    for (const name of ['States', 'Pops', 'Alerts', 'Military', 'Buildings']) {
      expect(screen.getByRole('button', { name })).toBeInTheDocument()
    }

    await user.click(screen.getByRole('button', { name: 'States' }))
    expect(screen.getByRole('heading', { name: 'States' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Ile-de-France' })).toBeInTheDocument()
    expect(window.location.hash).toBe('#/states')

    await user.click(screen.getByRole('button', { name: 'Military' }))
    expect(screen.getByRole('heading', { name: 'Military' })).toBeInTheDocument()
    expect(screen.getByRole('tab', { name: 'Army' })).toBeInTheDocument()
    expect(screen.getByRole('tab', { name: 'Navy' })).toBeInTheDocument()
    expect(screen.getByRole('tab', { name: 'Mobilization' })).toBeInTheDocument()
    expect(await screen.findByText('Armée du Nord')).toBeInTheDocument()
  })

  it('fills army, navy, and mobilization tabs from loaded_military', async () => {
    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)
    await selectSave(user)

    await user.click(screen.getByRole('button', { name: 'Military' }))
    expect(await screen.findByText('Armée du Nord')).toBeInTheDocument()
    expect(screen.getByText(/12,000 manpower/)).toBeInTheDocument()

    await user.click(screen.getByRole('tab', { name: 'Navy' }))
    expect(screen.getByRole('tab', { name: 'Navy' })).toHaveAttribute('aria-selected', 'true')
    expect(await screen.findByText('Atlantic Fleet')).toBeInTheDocument()
    expect(screen.queryByText('Armée du Nord')).not.toBeInTheDocument()
    expect(window.location.hash).toBe('#/military/navy')

    await user.click(screen.getByRole('tab', { name: 'Mobilization' }))
    expect(await screen.findByText('General Mobilization')).toBeInTheDocument()
    expect(window.location.hash).toBe('#/military/mobilization')
  })

  it('shows empty military lists with the snapshot limitation, not zero counts', async () => {
    const user = userEvent.setup()
    const api = mockApi()
    api.loaded_military = vi.fn(() =>
      JSON.stringify({
        armies: [],
        navies: [],
        mobilization: [],
        limitations: ['Military IR incomplete; missing managers yield empty lists'],
      }),
    )
    render(<App wasmApi={api} />)
    await selectSave(user)

    await user.click(screen.getByRole('button', { name: 'Military' }))
    expect(await screen.findByText('No armies recorded in this save.')).toBeInTheDocument()
    expect(
      screen.getByText('Military IR incomplete; missing managers yield empty lists'),
    ).toBeInTheDocument()
    expect(screen.queryByText(/0 armies/i)).not.toBeInTheDocument()

    await user.click(screen.getByRole('tab', { name: 'Mobilization' }))
    expect(await screen.findByText('None recorded')).toBeInTheDocument()
  })

  it('shows a state name on the States pane after a save loads', async () => {
    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)
    await selectSave(user)

    await user.click(screen.getByRole('button', { name: 'States' }))
    expect(await screen.findByRole('link', { name: 'Ile-de-France' })).toBeInTheDocument()
  })

  it('opens the Pops pane from the workbench nav', async () => {
    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)
    await selectSave(user)

    await user.click(screen.getByRole('button', { name: 'Pops' }))
    expect(screen.getByRole('heading', { name: 'Pops' })).toBeInTheDocument()
    expect(window.location.hash).toBe('#/pops')
  })

  it('shows grouped buildings and an optimizer', async () => {
    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)
    await selectSave(user)

    await user.click(screen.getByRole('button', { name: 'Buildings' }))
    expect(screen.getByRole('heading', { name: 'Buildings' })).toBeInTheDocument()
    expect(screen.getByRole('tab', { name: 'Overview' })).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByText('Rye Farms')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Optimize production methods' })).toBeEnabled()
    expect(window.location.hash).toBe('#/buildings')

    await user.click(screen.getByRole('button', { name: 'Expand Rye Farms' }))
    expect(screen.getByRole('link', { name: 'Ile-de-France' })).toHaveAttribute(
      'href',
      '#/buildings/building/9',
    )
    expect(screen.getAllByText('Simple Farming').length).toBeGreaterThan(0)
  })

  it('loads construction queues under Buildings → Queues', async () => {
    const user = userEvent.setup()
    const api = mockApi()
    render(<App wasmApi={api} />)
    await selectSave(user)

    await user.click(screen.getByRole('button', { name: 'Buildings' }))
    await user.click(screen.getByRole('tab', { name: 'Queues' }))
    expect(window.location.hash).toBe('#/buildings/queues')
    await waitFor(() => expect(api.loaded_constructions).toHaveBeenCalled())
    expect(await screen.findByText('Construction Sector')).toBeInTheDocument()
    await user.click(
      within(screen.getByRole('tablist', { name: 'Construction queue' })).getByRole('tab', {
        name: /^Private$/,
      }),
    )
    expect(screen.getByText('Logging Camp')).toBeInTheDocument()
  })

  it('opens a building page from a buildings hash', async () => {
    const user = userEvent.setup()
    window.location.hash = '#/buildings/building/9'
    render(<App wasmApi={mockApi()} />)
    await selectSave(user)

    expect(await screen.findByRole('heading', { name: 'Rye Farms' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Buildings' })).toHaveAttribute('href', '#/buildings')
    expect(screen.getByText('Workforce')).toBeInTheDocument()
  })

  it('opens a known pane from the location hash', async () => {
    window.location.hash = '#/military/navy'
    render(<App wasmApi={mockApi()} />)

    expect(await screen.findByRole('heading', { name: 'Military' })).toBeInTheDocument()
    expect(screen.getByRole('tab', { name: 'Navy' })).toHaveAttribute('aria-selected', 'true')
  })

  it('shows version, revision, and build time in the footer', () => {
    render(<App wasmApi={mockApi()} />)
    const footer = document.querySelector('.site-footer')
    expect(footer).toHaveTextContent(`Victoria 3 Analyzer v${__APP_VERSION__}`)
    expect(footer).toHaveTextContent('Built')
    expect(footer?.querySelector('time')).toHaveAttribute('dateTime')
  })

  it('names a downloaded save with origin, date, and step', async () => {
    const user = userEvent.setup()
    vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:save')
    vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {})
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {})
    render(<App wasmApi={mockApi()} />)
    await selectSave(user)
    await waitFor(async () => {
      expect((await loadStoredSave())?.save.name).toBe('campaign.v3')
    })

    await user.click(screen.getByRole('button', { name: 'Download' }))
    await waitFor(() => expect(click).toHaveBeenCalled())
    const anchor = click.mock.instances[0] as HTMLAnchorElement
    expect(anchor.download).toMatch(/^campaign_analyzer_1840\.2\.3_.+\.v3$/)
  })

  it('opens confirm from an alert Apply and shows Undo after commit', async () => {
    const user = userEvent.setup()
    const api = mockApi()
    api.loaded_alerts = vi.fn(() =>
      JSON.stringify({
        alerts: [
          {
            id: 'goods_shortage:grain',
            kind: 'goods_shortage',
            severity: 1,
            title: 'Grain shortage',
            summary: 'Need more grain.',
            good_name: 'grain',
            evidence: [],
            mitigations: [
              {
                id: 'build-rye',
                title: 'Add rye farm levels',
                detail: 'Build extra grain.',
                rank: 1,
                apply_ready: false,
                action: { type: 'build', building: 'building_rye_farm', extra_levels: 1 },
              },
            ],
          },
        ],
        limitations: [],
      }),
    )
    api.loaded_apply_delta = vi.fn(() =>
      JSON.stringify({
        ...JSON.parse(result),
        residual: 0.4,
        goods: [{ good_name: 'iron', base: 40, price: 40, buy: 120, sell: 100 }],
      }),
    )
    render(<App wasmApi={api} />)
    await selectSave(user)
    await waitFor(async () => {
      expect((await loadStoredSave())?.save.name).toBe('campaign.v3')
    })
    expect(screen.queryByRole('button', { name: 'Undo' })).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Alerts' }))
    await user.click(await screen.findByRole('link', { name: /Grain shortage/ }))
    expect(window.location.hash).toBe('#/prices/good/grain')
    await user.click(await screen.findByText('Grain shortage'))
    await user.click(screen.getByRole('button', { name: 'Apply' }))

    const dialog = await screen.findByRole('dialog', { name: 'Confirm apply' })
    expect(dialog).toHaveTextContent('0.00001')
    expect(dialog).toHaveTextContent('0.4')
    expect(dialog).toHaveTextContent('43.50')
    expect(dialog).toHaveTextContent('40.00')

    await user.click(screen.getByRole('button', { name: 'Confirm' }))
    await waitFor(() => expect(api.export_save).toHaveBeenCalled())
    expect(await screen.findByRole('button', { name: 'Undo' })).toBeInTheDocument()
    expect(screen.queryByRole('dialog', { name: 'Confirm apply' })).not.toBeInTheDocument()
  })
})
