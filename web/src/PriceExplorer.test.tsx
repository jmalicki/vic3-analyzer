import { cleanup, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { PriceExplorer } from './PriceExplorer'
import type { PricesResult, StateGood } from './types'

function stateGood(
  state_id: number,
  state_price: number,
  buy: number,
  sell: number,
  market_access = 1,
  market_price = 50,
): StateGood {
  const effective_mapi = 0.75 * market_access
  return {
    state_id,
    name: 'zany_tools',
    base: 40,
    buy,
    sell,
    market_price,
    state_price,
    market_access,
    effective_mapi,
    price: effective_mapi * market_price + (1 - effective_mapi) * state_price,
  }
}

const result: PricesResult = {
  scope: 'whole_save_synthetic',
  goods: [
    { name: 'zany_tools', label: 'Zany Tools', base: 40, price: 50, buy: 10, sell: 8 },
    { name: 'apples', label: 'Apples', base: 20, price: 15, buy: 5, sell: 7 },
  ],
  states: [
    { id: 2, region_id: 'STATE_ZEBRA', market_id: 1 },
    {
      id: 1,
      region_id: 'STATE_ALPACA',
      state_name: 'Alpaca',
      market_id: 1,
      arable_land: 10,
      infrastructure: 22,
      infrastructure_usage: 17,
    },
  ],
  state_goods: [
    stateGood(2, 70, 8, 1),
    stateGood(1, 30, 2, 7),
  ],
  buildings: [
    {
      id: 7,
      state_id: 1,
      type_id: 'building_silly_hammer_factory',
      level: 3,
      staffing: 2.4,
      production_method_ids: ['pm_goofy_hammers'],
      inputs: [{ name: 'iron', quantity: 2, value: 80 }],
      outputs: [{ name: 'zany_tools', quantity: 3, value: 150 }],
      revenue: 150,
      cost: 80,
      profit: 70,
      short_inputs: ['iron'],
      employees: [{ name: 'machinists', label: 'Machinists', count: 8000 }],
    },
  ],
  building_types: [
    { id: 'building_silly_hammer_factory', name: 'Silly Hammer Factory',
      group_id: 'bg_manufacturing',
    },
    { id: 'building_rye_farm', name: 'Rye Farms', group_id: 'bg_agriculture' },
  ],
  building_groups: [
    { id: 'bg_manufacturing', name: 'Manufacturing', category: 'urban', always_possible: false },
    { id: 'bg_agriculture', name: 'Agriculture',
      category: 'rural',
      land_usage: 'rural',
      always_possible: true,
      default_building: 'building_rye_farm',
    },
  ],
  state_pops: [
    {
      state_id: 1,
      profession_name: 'machinists',
      profession_label: 'Machinists',
      demand_size: 12000,
      workforce: 8000,
      dependents: 4000,
      literate: 2400,
      wealth: 14,
      culture_name: 'north_german',
      culture_label: 'North German',
      workplace_id: 7,
      needs: [
        {
          name: 'popneed_staple_foods',
          label: 'Staple foods',
          package_value: 10,
          goods: [{ name: 'apples', quantity: 4, value: 60 }],
        },
      ],
    },
  ],
  state_qualifications: [
    {
      state_id: 1,
      name: 'machinists',
      label: 'Machinists',
      qualified: 9000,
      employable: 8500,
      employed: 8000,
      jobs: 8000,
      shortage: 0,
    },
  ],
  state_needs: [
    {
      state_id: 1,
      name: 'popneed_staple_foods',
      label: 'Staple foods',
      package_value: 10,
      goods: [{ name: 'apples', quantity: 4, value: 60 }],
    },
  ],
  residual: 0,
  status: 'converged',
  limitations: [],
}

const scopedResult: PricesResult = {
  ...result,
  countries: [
    { id: 10, tag: 'ALP', name: 'Alpacania' },
    {
      id: 20,
      tag: 'BDG',
      name: 'Badgeria',
      flag_coa: 'BDG',
      flag_data_url: 'data:image/png;base64,FLAGBDG',
    },
  ],
  states: [
    { id: 1, region_id: 'STATE_ALPACA', country_id: 10, market_id: 1 },
    { id: 2, region_id: 'STATE_ZEBRA', country_id: 10, market_id: 2 },
    { id: 3, region_id: 'STATE_BADGER', country_id: 20, market_id: 1 },
    { id: 4, region_id: 'STATE_YAK', country_id: 20, market_id: 3 },
  ],
  state_goods: [
    stateGood(1, 35, 2, 7),
    stateGood(2, 70, 8, 1),
    stateGood(3, 45, 3, 4),
    stateGood(4, 55, 5, 6),
  ],
  buildings: [
    ...result.buildings!,
    {
      ...result.buildings![0],
      id: 8,
      state_id: 2,
      type_id: 'building_zebra_mill',
    },
  ],
}

describe('PriceExplorer', () => {
  beforeEach(() => {
    window.location.hash = ''
  })
  afterEach(cleanup)

  it('sorts goods by name, price, and percentage from base', async () => {
    const user = userEvent.setup()
    render(<PriceExplorer result={result} />)
    const body = screen.getAllByRole('rowgroup')[1]
    expect(within(body).getAllByRole('row')[0]).toHaveTextContent('Apples')

    await user.click(screen.getByRole('button', { name: 'Sort by Price' }))
    expect(within(body).getAllByRole('row')[0]).toHaveTextContent('Apples')
    await user.click(screen.getByRole('button', { name: 'Sort by Price' }))
    expect(within(body).getAllByRole('row')[0]).toHaveTextContent('Zany Tools')

    await user.click(screen.getByRole('button', { name: 'Sort by % from base price' }))
    expect(within(body).getAllByRole('row')[0]).toHaveTextContent('Apples')
  })

  it('shows the move from base price as a signed percentage with a direction arrow', () => {
    render(<PriceExplorer result={result} />)

    // 50 vs base price 40 is a quarter above; 15 vs base price 20 is a quarter below.
    expect(screen.getByText('▲')).toBeInTheDocument()
    expect(screen.getByText('+25.0%')).toBeInTheDocument()
    expect(screen.getByText('▼')).toBeInTheDocument()
    expect(screen.getByText('−25.0%')).toBeInTheDocument()
    expect(screen.getByText('+25.0%').closest('span')).toHaveAttribute(
      'title',
      '+10.00 vs base price 40.00',
    )
  })

  it('says a good is at base price rather than showing a bare zero', () => {
    render(
      <PriceExplorer
        result={{
          ...result,
          goods: [{ ...result.goods[0], price: result.goods[0].base }],
          state_goods: result.state_goods?.map((row) => ({ ...row, price: row.base })),
        }}
      />,
    )

    expect(screen.getByText('at base price')).toBeInTheDocument()
    expect(screen.queryByText('▲')).not.toBeInTheDocument()
  })

  /**
   * A big move on a cheap good outranks a small move on an expensive one, which
   * a currency-amount sort gets backwards.
   */
  it('ranks by relative move, not by currency amount', async () => {
    const user = userEvent.setup()
    render(
      <PriceExplorer
        result={{
          ...result,
          goods: [
            { name: 'cheap', label: 'Cheap', base: 10, price: 17, buy: 1, sell: 1 },
            { name: 'dear', label: 'Dear', base: 200, price: 220, buy: 1, sell: 1 },
          ],
        }}
      />,
    )

    await user.click(screen.getByRole('button', { name: 'Sort by % from base price' }))
    await user.click(screen.getByRole('button', { name: 'Sort by % from base price' }))
    const body = screen.getAllByRole('rowgroup')[1]
    expect(within(body).getAllByRole('row')[0]).toHaveTextContent('Cheap')
  })

  it('links a good to the Vic3-style state panel', async () => {
    const user = userEvent.setup()
    render(<PriceExplorer result={result} />)

    await user.click(screen.getByRole('link', { name: 'Zany Tools' }))
    expect(await screen.findByRole('heading', { name: 'Zany Tools by state' })).toBeInTheDocument()
    expect(screen.getByText(/base MAPI 75%/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Sort by State' })).toBeInTheDocument()

    await user.click(screen.getByRole('link', { name: 'Alpaca' }))
    expect(await screen.findByRole('heading', { name: 'Alpaca' })).toBeInTheDocument()
    expect(screen.getByRole('tab', { name: 'Overview' })).toHaveAttribute('aria-selected', 'true')
    for (const tab of ['Overview', 'Buildings', 'Population', 'Local Prices', 'Information']) {
      expect(screen.getByRole('tab', { name: tab })).toBeInTheDocument()
    }
    expect(screen.getByText('12,000')).toBeInTheDocument()

    await user.click(screen.getByRole('tab', { name: 'Buildings' }))
    expect(screen.getByRole('link', { name: 'Silly Hammer Factory' })).toBeInTheDocument()
    expect(screen.getByText('70.00')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: /Iron 2\.0/ })).toBeInTheDocument()

    await user.click(screen.getByRole('tab', { name: 'Population' }))
    expect(screen.getByRole('button', { name: 'Machinists' })).toBeInTheDocument()
    expect(screen.getByText('North German')).toBeInTheDocument()
    expect(screen.getAllByText('8,000').length).toBeGreaterThan(0)
    await user.click(screen.getByRole('button', { name: 'Machinists' }))
    expect(screen.getAllByText('Staple foods').length).toBeGreaterThan(0)
    expect(screen.getAllByRole('link', { name: 'Apples' }).length).toBeGreaterThan(0)
    expect(screen.getAllByText('4.0').length).toBeGreaterThan(0)
  })

  it('shows distinct locally attributed prices for each state', () => {
    window.location.hash = '#/prices/good/zany_tools'
    render(<PriceExplorer result={result} />)

    expect(screen.getByRole('button', { name: 'Sort by State price' })).toBeInTheDocument()
    const zebra = screen.getByRole('link', { name: 'Zebra' }).closest('tr')
    const alpaca = screen.getByRole('link', { name: 'Alpaca' }).closest('tr')
    expect(zebra).toHaveTextContent('55.00')
    expect(alpaca).toHaveTextContent('45.00')
  })

  it('applies scope to global order-weighted average prices and orders', async () => {
    const user = userEvent.setup()
    render(<PriceExplorer result={scopedResult} playerCountryId={10} playerMarketId={1} />)

    const globalRow = () => screen.getByRole('link', { name: 'Zany Tools' }).closest('tr')
    // Our market: states 1 and 3, weighted by each state's buy + sell orders.
    expect(globalRow()).toHaveTextContent('47.34')
    expect(globalRow()).toHaveTextContent('5.00')
    expect(globalRow()).toHaveTextContent('11.00')

    await user.click(screen.getByRole('button', { name: 'Domestic' }))
    // Domestic: states 1 and 2.
    expect(globalRow()).toHaveTextContent('50.63')
    expect(globalRow()).toHaveTextContent('10.00')
    expect(globalRow()).toHaveTextContent('8.00')
  })

  it('defaults to our market and hides states in foreign markets', () => {
    window.location.hash = '#/prices/good/zany_tools'
    render(
      <PriceExplorer result={scopedResult} playerCountryId={10} playerMarketId={1} />,
    )

    expect(screen.getByRole('button', { name: 'Our market' })).toHaveAttribute(
      'aria-pressed',
      'true',
    )
    expect(screen.getByRole('link', { name: 'Alpaca' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Badger' })).toBeInTheDocument()
    expect(screen.queryByRole('link', { name: 'Zebra' })).not.toBeInTheDocument()
    expect(screen.queryByRole('link', { name: 'Yak' })).not.toBeInTheDocument()
  })

  it('switches list scope and keeps state pages unfiltered', async () => {
    const user = userEvent.setup()
    window.location.hash = '#/prices/good/zany_tools'
    render(
      <PriceExplorer result={scopedResult} playerCountryId={10} playerMarketId={1} />,
    )

    await user.click(screen.getByRole('button', { name: 'Domestic' }))
    expect(screen.getByRole('link', { name: 'Alpaca' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Zebra' })).toBeInTheDocument()
    expect(screen.queryByRole('link', { name: 'Badger' })).not.toBeInTheDocument()

    await user.click(screen.getByRole('link', { name: 'Zebra' }))
    await user.click(screen.getByRole('tab', { name: 'Buildings' }))
    expect(await screen.findByText('Zebra Mill')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Our market' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Domestic' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'All' })).not.toBeInTheDocument()
  })

  it('shows remaining arable land as honest rural capacity', async () => {
    const user = userEvent.setup()
    window.location.hash = '#/prices/state/1'
    render(<PriceExplorer result={result} />)

    await user.click(screen.getByRole('tab', { name: 'Buildings' }))
    expect(screen.getByText('10 empty rural slots')).toBeInTheDocument()
    expect(screen.getByText('Rye Farms')).toBeInTheDocument()
    expect(screen.getByText(/constructable placeholder/)).toBeInTheDocument()
  })

  it('opens a building detail route with workforce and linked goods', async () => {
    const user = userEvent.setup()
    window.location.hash = '#/prices/state/1'
    render(<PriceExplorer result={result} />)

    await user.click(screen.getByRole('tab', { name: 'Buildings' }))
    await user.click(screen.getByRole('link', { name: 'Silly Hammer Factory' }))
    expect(await screen.findByRole('heading', { name: 'Silly Hammer Factory' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Alpaca' })).toBeInTheDocument()
    expect(screen.getByText('Workforce')).toBeInTheDocument()
    expect(screen.getByText('Machinists')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: /Zany Tools 3\.0/ })).toBeInTheDocument()
  })

  it('lists this state local prices with good links', async () => {
    const user = userEvent.setup()
    window.location.hash = '#/prices/state/1'
    render(<PriceExplorer result={result} />)

    await user.click(screen.getByRole('tab', { name: 'Local Prices' }))
    expect(screen.getByRole('link', { name: 'Zany Tools' })).toBeInTheDocument()
    expect(screen.getByText('45.00')).toBeInTheDocument()
  })

  it('parks capacity facts on Information', async () => {
    const user = userEvent.setup()
    window.location.hash = '#/prices/state/1'
    render(<PriceExplorer result={result} />)

    await user.click(screen.getByRole('tab', { name: 'Information' }))
    expect(screen.getByText('Arable land')).toBeInTheDocument()
    expect(screen.getByText('10')).toBeInTheDocument()
    expect(screen.getByText(/incorporation/)).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Goods' })).toHaveAttribute('href', '#/prices')
  })

  it('opens the same state panel from a states hash with a States breadcrumb', () => {
    window.location.hash = '#/states/1'
    render(<PriceExplorer result={result} />)

    expect(screen.getByRole('heading', { name: 'Alpaca' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'States' })).toHaveAttribute('href', '#/states')
    expect(screen.queryByRole('link', { name: 'Goods' })).not.toBeInTheDocument()
  })

  it('opens the same building page from a buildings hash', () => {
    window.location.hash = '#/buildings/building/7'
    render(<PriceExplorer result={result} />)

    expect(screen.getByRole('heading', { name: 'Silly Hammer Factory' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Buildings' })).toHaveAttribute('href', '#/buildings')
    expect(screen.getByText('Workforce')).toBeInTheDocument()
    expect(screen.queryByRole('link', { name: 'Goods' })).not.toBeInTheDocument()
  })

  it('can return to Our market when playerMarketId is present', async () => {
    const user = userEvent.setup()
    window.location.hash = '#/prices/good/zany_tools'
    render(<PriceExplorer result={scopedResult} playerCountryId={10} playerMarketId={1} />)

    await user.click(screen.getByRole('button', { name: 'Domestic' }))
    await user.click(screen.getByRole('button', { name: 'Our market' }))
    expect(screen.getByRole('button', { name: 'Our market' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByRole('link', { name: 'Badger' })).toBeInTheDocument()
    expect(screen.queryByRole('link', { name: 'Zebra' })).not.toBeInTheDocument()
  })

  it('shows every state in all scope', async () => {
    const user = userEvent.setup()
    window.location.hash = '#/prices/good/zany_tools'
    render(
      <PriceExplorer result={scopedResult} playerCountryId={10} playerMarketId={1} />,
    )

    await user.click(screen.getByRole('button', { name: 'All' }))
    for (const state of ['Alpaca', 'Zebra', 'Badger', 'Yak']) {
      expect(screen.getByRole('link', { name: state })).toBeInTheDocument()
    }
  })

  it('falls back to all states when player market identity is missing', () => {
    window.location.hash = '#/prices/good/zany_tools'
    render(<PriceExplorer result={scopedResult} />)

    expect(screen.getByText('Player market unavailable; showing all states.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'All' })).toHaveAttribute(
      'aria-pressed',
      'true',
    )
    for (const state of ['Alpaca', 'Zebra', 'Badger', 'Yak']) {
      expect(screen.getByRole('link', { name: state })).toBeInTheDocument()
    }
  })

  it('shows a foreign country flag with a localized name tooltip', () => {
    window.location.hash = '#/prices/good/zany_tools'
    const { container } = render(
      <PriceExplorer result={scopedResult} playerCountryId={10} playerMarketId={1} />,
    )

    const flag = container.querySelector('img.country-flag')
    expect(flag).toHaveAttribute('src', 'data:image/png;base64,FLAGBDG')
    expect(flag).toHaveAttribute('title', 'Badgeria')
    expect(container.querySelectorAll('img.country-flag')).toHaveLength(1)
  })

  it('shows a game icon beside a good that has one', () => {
    const icons = { apples: 'data:image/png;base64,APPLESICON' }
    const { container } = render(<PriceExplorer result={result} icons={icons} />)

    const rendered = container.querySelectorAll('img.good-icon')
    expect(rendered).toHaveLength(1)
    expect(rendered[0]).toHaveAttribute('src', icons.apples)
    // Decorative: the good name beside it already carries the meaning.
    expect(rendered[0]).toHaveAttribute('alt', '')
  })

  it('renders goods without icons when the blob carries none', () => {
    const { container } = render(<PriceExplorer result={result} />)

    expect(container.querySelectorAll('img.good-icon')).toHaveLength(0)
    expect(screen.getByRole('link', { name: 'Apples' })).toBeInTheDocument()
  })

  it('explains a market where nothing placed an order', () => {
    render(
      <PriceExplorer
        result={{
          ...result,
          goods: result.goods.map((good) => ({ ...good, price: good.base, buy: 0, sell: 0 })),
          inputs: {
            pops: 0,
            skipped_pops: 41234,
            buildings: 12,
            skipped_buildings: 0,
            buildings_without_method: 12,
            buildings_without_orders: 12,
            goods_with_orders: 0,
          },
        }}
      />,
    )

    const warning = screen.getByRole('status')
    expect(warning).toHaveTextContent('No buy or sell orders were reconstructed')
    expect(warning).toHaveTextContent(
      '41,234 pops in the save were missing workforce/dependents (or legacy population fields) or wealth',
    )
    expect(warning).toHaveTextContent('none of the 12 buildings had saved goods IO')
  })

  it('stays quiet when the market has orders', () => {
    render(
      <PriceExplorer
        result={{
          ...result,
          inputs: {
            pops: 5,
            skipped_pops: 0,
            buildings: 1,
            skipped_buildings: 0,
            buildings_without_method: 0,
            buildings_without_orders: 0,
            goods_with_orders: 2,
          },
        }}
      />,
    )

    expect(screen.queryByRole('status')).not.toBeInTheDocument()
  })

  it('shows local recommendations on a good page', async () => {
    const user = userEvent.setup()
    window.location.hash = '#/prices/good/zany_tools'
    render(
      <PriceExplorer
        result={result}
        alerts={[
          {
            id: 'goods_shortage:zany_tools',
            kind: 'goods_shortage',
            severity: 1,
            title: 'Zany tools shortage',
            summary: 'Buy exceeds sell.',
            good_name: 'zany_tools',
            evidence: [],
            mitigations: [
              {
                id: 'build',
                title: 'Add factory levels',
                detail: 'Produce more tools.',
                rank: 1,
                apply_ready: false,
              },
            ],
          },
        ]}
      />,
    )

    expect(screen.getByRole('heading', { name: 'Recommendations' })).toBeInTheDocument()
    await user.click(screen.getByText('Zany tools shortage'))
    expect(screen.getByText(/Add factory levels/)).toBeInTheDocument()
  })
})
