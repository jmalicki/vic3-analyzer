import { cleanup, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { BuildingsPane } from './BuildingsPane'
import type { PricesResult, ProductionMethodDef } from './types'
import type { WasmApi } from './wasm'

const dummyName =
  '#This is a dummy building that serve no gameplay mechanical purpose but still need to be reacted to by city hub graphics. It (and this text) should never show up in the UI#!'

const result: PricesResult = {
  goods: [
    { name: 'iron', label: 'Iron', base: 40, price: 50, buy: 2, sell: 1 },
    { name: 'grain', label: 'Grain', base: 20, price: 18, buy: 4, sell: 8 },
  ],
  states: [
    { id: 1, region_name: 'STATE_ALPACA', label: 'Alpaca', country_id: 10, market_id: 1 },
    { id: 2, region_name: 'STATE_ZEBRA', label: 'Zebra', country_id: 10, market_id: 2 },
    { id: 3, region_name: 'STATE_BADGER', label: 'Badger', country_id: 20, market_id: 3 },
  ],
  buildings: [
    {
      id: 7,
      state_id: 1,
      building_type_name: 'building_silly_hammer_factory',
      level: 3,
      staffing: 2.4,
      production_method_ids: ['pm_goofy_hammers'],
      inputs: [{ name: 'iron', quantity: 2, value: 80 }],
      outputs: [{ name: 'zany_tools', quantity: 3, value: 150 }],
      revenue: 150,
      cost: 80,
      profit: 70,
      short_inputs: ['iron'],
    },
    {
      id: 8,
      state_id: 2,
      building_type_name: 'building_silly_hammer_factory',
      level: 1,
      staffing: 1,
      production_method_ids: ['pm_steam_hammers'],
      inputs: [],
      outputs: [],
      revenue: 10,
      cost: 5,
      profit: 5,
      short_inputs: [],
    },
    {
      id: 9,
      state_id: 1,
      building_type_name: 'building_rye_farm',
      level: 10,
      staffing: 8,
      production_method_ids: ['pm_simple_farming'],
      inputs: [],
      outputs: [{ name: 'grain', quantity: 20, value: 200 }],
      revenue: 200,
      cost: 20,
      profit: 180,
      short_inputs: [],
    },
    {
      id: 10,
      state_id: 1,
      building_type_name: 'building_city_hub_dummy',
      level: 1,
      staffing: 0,
      production_method_ids: [],
      inputs: [],
      outputs: [],
      revenue: 0,
      cost: 0,
      profit: 0,
      short_inputs: [],
    },
    {
      id: 11,
      state_id: 3,
      building_type_name: 'building_badger_mill',
      level: 2,
      staffing: 2,
      production_method_ids: ['pm_goofy_hammers'],
      inputs: [{ name: 'iron', quantity: 1, value: 40 }],
      outputs: [{ name: 'zany_tools', quantity: 1, value: 50 }],
      revenue: 50,
      cost: 40,
      profit: 10,
      short_inputs: [],
    },
  ],
  building_types: [
    { id: 0, name: 'building_silly_hammer_factory', label: 'Silly Hammer Factory' },
    { id: 1, name: 'building_rye_farm', label: 'Rye Farms' },
    { id: 2, name: 'building_city_hub_dummy', label: dummyName },
    { id: 3, name: 'building_badger_mill', label: 'Badger Mill' },
  ],
  residual: 0,
  status: 'converged',
  limitations: [],
}

const methods: ProductionMethodDef[] = [
  { id: 'pm_goofy_hammers', inputs: [{ good: 'iron', qty: 2 }], outputs: [{ good: 'zany_tools', qty: 3 }] },
  { id: 'pm_steam_hammers', inputs: [{ good: 'iron', qty: 3 }], outputs: [{ good: 'zany_tools', qty: 5 }] },
  { id: 'pm_simple_farming', inputs: [], outputs: [{ good: 'grain', qty: 20 }] },
]

function renderBuildings(
  props: {
    result?: PricesResult
    productionMethods?: ProductionMethodDef[]
    api?: WasmApi
    onWhatIf?: (building: string, extraLevels: number) => void
    onApply?: (delta: { extra_levels?: unknown; production_methods?: unknown }) => void
    playerCountryId?: number
    playerMarketId?: number
  } = {},
) {
  const { result: prices = result, ...rest } = props
  return render(
    <BuildingsPane result={prices} playerCountryId={10} playerMarketId={1} {...rest} />,
  )
}

function typeNames(): string[] {
  return [...document.querySelectorAll('.buildings-table > tbody > tr > th')].map((cell) =>
    cell.textContent?.replace(/^[▶▼]\s*/, '').trim() ?? '',
  )
}

describe('BuildingsPane', () => {
  beforeEach(() => {
    window.location.hash = '#/buildings'
  })
  afterEach(cleanup)

  it('groups buildings by type and sorts by name, profit, and shortage', async () => {
    const user = userEvent.setup()
    renderBuildings()

    expect(screen.getByRole('heading', { name: 'Buildings' })).toBeInTheDocument()
    expect(typeNames()[0]).toContain('Rye Farms')
    expect(typeNames()[1]).toContain('Silly Hammer Factory')

    await user.click(screen.getByRole('button', { name: 'Sort by Profit' }))
    expect(typeNames()[0]).toContain('Silly Hammer Factory')
    expect(typeNames()[1]).toContain('Rye Farms')

    await user.click(screen.getByRole('button', { name: 'Sort by Shortage' }))
    expect(typeNames()[0]).toContain('Rye Farms')
    expect(typeNames()[1]).toContain('Silly Hammer Factory')
  })

  it('defaults to Domestic and hides foreign and dummy types', () => {
    renderBuildings()

    expect(screen.getByRole('button', { name: 'Domestic' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByText('Rye Farms')).toBeInTheDocument()
    expect(screen.getByText('Silly Hammer Factory')).toBeInTheDocument()
    expect(screen.queryByText('Badger Mill')).not.toBeInTheDocument()
    expect(screen.queryByText(/should never show up in the UI/)).not.toBeInTheDocument()
    expect(document.querySelector('.buildings-table-scroll')).toBeTruthy()
  })

  it('shows the player market and all countries from the scope filter', async () => {
    const user = userEvent.setup()
    renderBuildings()

    await user.click(screen.getByRole('button', { name: 'Our market' }))
    expect(screen.getByText('Rye Farms')).toBeInTheDocument()
    expect(screen.getByText('Silly Hammer Factory')).toBeInTheDocument()
    expect(screen.queryByText('Badger Mill')).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Expand Silly Hammer Factory' }))
    expect(screen.getByRole('link', { name: 'Alpaca' })).toBeInTheDocument()
    expect(screen.queryByRole('link', { name: 'Zebra' })).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'All' }))
    expect(screen.getByText('Badger Mill')).toBeInTheDocument()
    expect(screen.queryByText(/should never show up in the UI/)).not.toBeInTheDocument()
  })

  it('falls back to all buildings when the player country is missing', () => {
    renderBuildings({ playerCountryId: undefined, playerMarketId: undefined })

    expect(screen.getByText(/Player country unavailable/)).toBeInTheDocument()
    expect(screen.getByText('Badger Mill')).toBeInTheDocument()
    expect(screen.queryByText(/should never show up in the UI/)).not.toBeInTheDocument()
  })

  it('expands a type to instance rows with production methods', async () => {
    const user = userEvent.setup()
    renderBuildings()

    await user.click(screen.getByRole('button', { name: 'Expand Silly Hammer Factory' }))
    expect(screen.getByRole('link', { name: 'Alpaca' })).toHaveAttribute('href', '#/buildings/building/7')
    expect(screen.getByRole('link', { name: 'Zebra' })).toHaveAttribute('href', '#/buildings/building/8')
    expect(screen.getAllByText('Goofy Hammers').length).toBeGreaterThan(0)
  })

  it('opens a building page from a deep link', () => {
    window.location.hash = '#/buildings/building/7'
    renderBuildings()

    expect(screen.getByRole('heading', { name: 'Silly Hammer Factory' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Buildings' })).toHaveAttribute('href', '#/buildings')
    expect(screen.getByText('Workforce')).toBeInTheDocument()
  })

  it('keeps the optimizer disabled without an API', () => {
    renderBuildings()

    const optimizer = screen.getByRole('button', { name: 'Optimize production methods' })
    expect(optimizer).toBeDisabled()
  })

  it('shows grouped optimizer changes from the mock API', async () => {
    const user = userEvent.setup()
    const api = {
      loaded_optimize_pms: vi.fn(() =>
        JSON.stringify({
          axis: 'income',
          changes: [
            {
              building_type: 'building_rye_farm',
              building_id: 9,
              from: ['pm_simple_farming'],
              to: ['pm_soil_enriching_farming'],
            },
          ],
          delta: { income: 12.5, productivity: 1.2, sol: 0, residual: -0.001 },
          limitations: [],
          world_delta: {
            production_methods: [{ building_id: 9, methods: ['pm_soil_enriching_farming'] }],
          },
        }),
      ),
    }
    renderBuildings({ api: api as unknown as WasmApi })

    await user.selectOptions(screen.getByRole('combobox', { name: 'Optimization axis' }), 'income')
    await user.click(screen.getByRole('button', { name: 'Optimize production methods' }))

    expect(api.loaded_optimize_pms).toHaveBeenCalledWith('{"axis":"income"}')
    expect(screen.getByText(/Estimated Δ: income \+12\.50/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Expand Rye Farms changes' }))
    expect(screen.getByText(/Simple Farming → Soil Enriching Farming/)).toBeInTheDocument()
    const apply = screen.getAllByRole('button', { name: 'Apply' })
    expect(apply.length).toBeGreaterThan(0)
    for (const button of apply) {
      expect(button).toBeDisabled()
    }
  })

  it('applies optimizer world_delta when onApply is set', async () => {
    const user = userEvent.setup()
    const onApply = vi.fn()
    const api = {
      loaded_optimize_pms: vi.fn(() =>
        JSON.stringify({
          axis: 'income',
          changes: [
            {
              building_type: 'building_rye_farm',
              building_id: 9,
              from: ['pm_simple_farming'],
              to: ['pm_soil_enriching_farming'],
            },
          ],
          delta: { income: 12.5, productivity: 1.2, sol: 0, residual: -0.001 },
          limitations: [],
          world_delta: {
            production_methods: [{ building_id: 9, methods: ['pm_soil_enriching_farming'] }],
          },
        }),
      ),
    }
    renderBuildings({ api: api as unknown as WasmApi, onApply })
    await user.click(screen.getByRole('button', { name: 'Optimize production methods' }))
    await user.click(screen.getByRole('button', { name: 'Apply all' }))
    expect(onApply).toHaveBeenCalledWith({
      production_methods: [{ building_id: 9, methods: ['pm_soil_enriching_farming'] }],
    })
  })

  it('lists only production methods used on that building type', async () => {
    const user = userEvent.setup()
    renderBuildings({ productionMethods: methods })

    await user.click(screen.getByRole('button', { name: 'Expand Silly Hammer Factory' }))
    const alpaca = screen.getByRole('checkbox', { name: 'Goofy Hammers for Alpaca' })
    expect(alpaca).toBeChecked()
    expect(screen.getByRole('checkbox', { name: 'Steam Hammers for Alpaca' })).not.toBeChecked()
    expect(screen.queryByRole('checkbox', { name: 'Simple Farming for Alpaca' })).not.toBeInTheDocument()
    expect(screen.getAllByText(/Preview uses selected methods/).length).toBeGreaterThan(0)

    await user.click(screen.getByRole('checkbox', { name: 'Steam Hammers for Alpaca' }))
    const detail = screen.getByRole('link', { name: 'Alpaca' }).closest('tr')
    expect(detail).toBeTruthy()
    expect(within(detail as HTMLElement).getByText(/Zany Tools 8/)).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Collapse Silly Hammer Factory' }))
    await user.click(screen.getByRole('button', { name: 'Expand Rye Farms' }))
    expect(screen.getByRole('checkbox', { name: 'Simple Farming for Alpaca' })).toBeChecked()
    expect(screen.queryByRole('checkbox', { name: 'Goofy Hammers for Alpaca' })).not.toBeInTheDocument()
  })

  it('runs extra-level what-if from the type row', async () => {
    const user = userEvent.setup()
    const onWhatIf = vi.fn()
    renderBuildings({ onWhatIf })

    await user.clear(screen.getByRole('spinbutton', { name: 'Extra levels for Rye Farms' }))
    await user.type(screen.getByRole('spinbutton', { name: 'Extra levels for Rye Farms' }), '3')
    await user.click(screen.getAllByRole('button', { name: 'Run what-if' })[0])
    expect(onWhatIf).toHaveBeenCalledWith('building_rye_farm', 3)
  })

  it('applies extra levels and changed production methods', async () => {
    const user = userEvent.setup()
    const onApply = vi.fn()
    renderBuildings({ productionMethods: methods, onApply })

    await user.click(screen.getByRole('button', { name: 'Apply extra levels for Rye Farms' }))
    expect(onApply).toHaveBeenCalledWith({
      extra_levels: [{ building_type_id: 1, extra_levels: 1 }],
    })

    await user.click(screen.getByRole('button', { name: 'Expand Silly Hammer Factory' }))
    expect(screen.getByRole('button', { name: 'Apply production methods for Alpaca' })).toBeDisabled()
    await user.click(screen.getByRole('checkbox', { name: 'Steam Hammers for Alpaca' }))
    await user.click(screen.getByRole('button', { name: 'Apply production methods for Alpaca' }))
    expect(onApply).toHaveBeenCalledWith({
      production_methods: [{ building_id: 7, methods: ['pm_goofy_hammers', 'pm_steam_hammers'] }],
    })
  })
})
