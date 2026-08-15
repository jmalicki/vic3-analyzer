import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
import { clearAnalyses, listAnalyses, saveAnalysis } from './archive'
import type { AnalysisRecord } from './types'
import type { WasmApi } from './wasm'

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
    parse_save: vi.fn(() => JSON.stringify({ tag: 'FRA', date: '1840.2.3', version: '1.9.0' })),
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
  await screen.findByText('Using the bundled demo definitions blob.')
}

describe('prices UI', () => {
  beforeEach(async () => {
    await clearAnalyses()
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

    expect(await screen.findByText('iron')).toBeInTheDocument()
    expect(screen.getByText('43.50')).toBeInTheDocument()
    expect(screen.getByText('Employment and production methods stay frozen.')).toBeInTheDocument()
    await waitFor(async () => expect(await listAnalyses()).toHaveLength(1))
    expect(api.prices).toHaveBeenCalled()
  })

  it('builds and submits the what-if form from the wasm schema', async () => {
    const user = userEvent.setup()
    const api = mockApi()
    render(<App wasmApi={api} />)
    await selectSave(user)

    await user.type(screen.getByLabelText('Building'), 'building_steel_mills')
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

    await user.clear(screen.getByLabelText('Gaps goal'))
    await user.type(screen.getByLabelText('Gaps goal'), 'research(tech=nitroglycerin)')
    await user.click(screen.getByRole('button', { name: 'Run gaps' }))

    expect(await screen.findByText('Satisfied: No')).toBeInTheDocument()
    expect(screen.getByText('{"HasTech":"nitroglycerin"}')).toBeInTheDocument()
    expect(
      screen.getByText('{"GoodPrice":{"good":"ammunition","rel":"Le","value":40}}'),
    ).toBeInTheDocument()
    expect(screen.getByText('Solvent')).toBeInTheDocument()
    expect(screen.getByText('Goal gaps use prices from a frozen-world solve.')).toBeInTheDocument()
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

    await user.clear(screen.getByLabelText('Plan goal'))
    await user.type(screen.getByLabelText('Plan goal'), 'research(tech=nitroglycerin)')
    await user.type(screen.getByLabelText('Plan label'), 'rush')
    await user.click(screen.getByRole('button', { name: 'Run plan' }))

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

  it('uses bundled defs by default and lets a custom blob override them', async () => {
    const user = userEvent.setup()
    const api = mockApi()
    render(<App wasmApi={api} />)
    await selectSave(user)

    await user.upload(screen.getByLabelText('Choose definitions blob'), new File(['custom'], 'custom.postcard'))
    expect(await screen.findByText('Using your file: custom.postcard')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Analyze prices' }))
    await waitFor(() => expect(api.prices).toHaveBeenCalled())
    const defsArg = vi.mocked(api.prices).mock.calls[0][2]
    expect(Array.from(defsArg)).toEqual([99, 117, 115, 116, 111, 109]) // "custom"
  })

  it('shows a missing-bundled-defs message when the fixture cannot load', async () => {
    mockBundledDefs(false)
    render(<App wasmApi={mockApi()} />)
    expect(
      await screen.findByText(/Bundled demo definitions are unavailable/),
    ).toBeInTheDocument()
  })

  it('uses the remembered Chromium picker when available', async () => {
    const picked = new File(['ironman'], 'ironman.v3')
    const showOpenFilePicker = vi.fn(async () => [{ getFile: async () => picked }])
    vi.stubGlobal('showOpenFilePicker', showOpenFilePicker)

    const user = userEvent.setup()
    render(<App wasmApi={mockApi()} />)
    await screen.findByText('Using the bundled demo definitions blob.')
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
    expect(screen.getByText(/cannot open that path automatically/)).toBeInTheDocument()
    expect(document.querySelector('.path-hint-path')?.textContent).toMatch(
      /Paradox Interactive[/\\]Victoria 3[/\\]save games/,
    )
  })
})
