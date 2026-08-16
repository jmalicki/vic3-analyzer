import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
import { clearAnalyses, listAnalyses, saveAnalysis } from './archive'
import { clearStoredDefs, storeDefs } from './defsStore'
import type { AnalysisRecord } from './types'
import type { WasmApi } from './wasm'

/** Test-only blobs; silly names so they never look like a real install export. */
const result = JSON.stringify({
  goods: [{ id: 'iron', base: 40, price: 43.5, buy: 120, sell: 100 }],
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
    build_defs_blob: vi.fn(() => new Uint8Array([7, 8, 9])),
    defs_icons: vi.fn(() => JSON.stringify({ iron: 'data:image/png;base64,IRONICON' })),
    defs_summary: vi.fn(() =>
      JSON.stringify({
        blob_version: 5,
        goods: 3,
        labels: 3,
        icons: 1,
        production_methods: 5,
        pop_needs: 2,
        buy_packages: 1,
        price_range: 0.75,
      }),
    ),
    parse_save: vi.fn(() =>
      JSON.stringify({
        tag: 'FRA',
        country_id: 16777216,
        market_id: 1,
        date: '1840.2.3',
        version: '1.9.0',
        buildings: ['building_rye_farm', 'building_steel_mills'],
      }),
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
    await clearAnalyses()
    await clearStoredDefs()
    mockBundledDefs()
  })
  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
  })

  it('renders mocked wasm goods and limitations, then archives the run', async () => {
    const user = userEvent.setup()
    const api = mockApi()
    render(<App wasmApi={api} />)
    await selectSave(user)

    await user.click(screen.getByRole('button', { name: 'Analyze prices' }))

    expect(await screen.findByText('Iron')).toBeInTheDocument()
    expect(screen.getByText('43.50')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Method and limitations' })).toBeInTheDocument()
    await waitFor(async () => expect(await listAnalyses()).toHaveLength(1))
    expect(api.prices).toHaveBeenCalled()
  })

  it('shows the game icon for a priced good', async () => {
    const user = userEvent.setup()
    const { container } = render(<App wasmApi={mockApi()} />)
    await selectSave(user)

    await user.click(screen.getByRole('button', { name: 'Analyze prices' }))
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
        blob_version: 5,
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

  it('clears a stale price table when the definitions change', async () => {
    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)
    await selectSave(user)
    await user.click(screen.getByRole('button', { name: 'Analyze prices' }))
    expect(await screen.findByText('Iron')).toBeInTheDocument()

    await buildDefinitions(user)

    await waitFor(() => expect(screen.queryByText('Iron')).not.toBeInTheDocument())
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
      expect(api.what_if).toHaveBeenCalledWith(
        expect.any(Uint8Array),
        undefined,
        expect.any(Uint8Array),
        '{}',
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
    expect(api.gaps).toHaveBeenCalledWith(
      expect.any(Uint8Array),
      undefined,
      expect.any(Uint8Array),
      '{}',
      'research(tech=nitroglycerin)',
    )
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
    await waitFor(() => expect(api.plan).toHaveBeenCalled())
    const records = await listAnalyses()
    expect(records[0]).toMatchObject({ kind: 'plan', label: 'rush' })
    expect(records[0].result).toMatchObject({ day_cost: 365 })
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
    api.build_defs_blob = vi.fn(() => new TextEncoder().encode('MOCKY-NOT-A-REAL-BLOB'))
    render(<App wasmApi={api} />)
    await selectSave(user)

    await buildDefinitions(user)
    expect(await screen.findByText(/Using your file: defs\.postcard/)).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Analyze prices' }))
    await waitFor(() => expect(api.prices).toHaveBeenCalled())
    const defsArg = vi.mocked(api.prices).mock.calls[0][2]
    // Exact builder output proves prices got the locally built definitions, not the demo fixture.
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

  it('clears a stored blob when Rust rejects its format version', async () => {
    await storeDefs(new File(['OLD-V1-DEFS'], 'ancient-v1.postcard'))
    const api = mockApi()
    api.defs_summary = vi.fn((defs) => {
      if (new TextDecoder().decode(defs) === 'OLD-V1-DEFS') {
        throw new Error('defs blob version 1 is not supported (expected 5)')
      }
      return JSON.stringify({
        blob_version: 5,
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
      'defs blob version 1 is not supported (expected 5). Rebuild definitions from your Victoria 3 game folder for this app version.',
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

  it('greys out analysis tools and names every missing input', async () => {
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockRejectedValue(new Error('no defs'))
    render(<App wasmApi={mockApi()} />)

    expect(
      await screen.findByText(/Analysis needs a \.v3 save and game definitions/),
    ).toBeInTheDocument()
    await waitFor(() =>
      expect(document.querySelector('.workspace-page')).toHaveClass('needs-defs'),
    )
    expect(screen.getByRole('button', { name: 'Analyze prices' })).toBeDisabled()
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

  it('shows version, revision, and build time in the footer', () => {
    render(<App wasmApi={mockApi()} />)
    const footer = document.querySelector('.site-footer')
    expect(footer).toHaveTextContent('vic3-analyzer v0.1.0')
    expect(footer).toHaveTextContent('Built')
    expect(footer?.querySelector('time')).toHaveAttribute('dateTime')
  })
})
