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

async function selectFiles(user: ReturnType<typeof userEvent.setup>) {
  await user.upload(screen.getByLabelText('Save file'), new File(['save'], 'campaign.v3'))
  await user.upload(screen.getByLabelText('Definitions blob'), new File(['defs'], 'defs.postcard'))
  await screen.findByText('FRA')
}

describe('prices UI', () => {
  beforeEach(clearAnalyses)
  afterEach(cleanup)

  it('renders mocked wasm goods and limitations, then archives the run', async () => {
    const user = userEvent.setup()
    const api = mockApi()
    render(<App wasmApi={api} />)
    await selectFiles(user)

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
    await selectFiles(user)

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
    await selectFiles(user)

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
    await selectFiles(user)

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
})
