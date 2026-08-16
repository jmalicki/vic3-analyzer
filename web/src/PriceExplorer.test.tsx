import { cleanup, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { PriceExplorer } from './PriceExplorer'
import type { PricesResult } from './types'

const result: PricesResult = {
  scope: 'whole_save_synthetic',
  goods: [
    { id: 'zany_tools', name: 'Zany Tools', base: 40, price: 50, buy: 10, sell: 8 },
    { id: 'apples', name: 'Apples', base: 20, price: 15, buy: 5, sell: 7 },
  ],
  states: [
    { id: 2, region_id: 'STATE_ZEBRA', market_id: 1 },
    { id: 1, region_id: 'STATE_ALPACA', market_id: 1 },
  ],
  state_goods: [
    { state_id: 2, good_id: 'zany_tools', base: 40, price: 50, buy: 8, sell: 1 },
    { state_id: 1, good_id: 'zany_tools', base: 40, price: 50, buy: 2, sell: 7 },
  ],
  buildings: [
    {
      id: 7,
      state_id: 1,
      type_id: 'building_silly_hammer_factory',
      level: 3,
      staffing: 0.8,
      production_method_ids: ['pm_goofy_hammers'],
      inputs: [{ good_id: 'iron', quantity: 2, value: 80 }],
      outputs: [{ good_id: 'zany_tools', quantity: 3, value: 150 }],
      revenue: 150,
      cost: 80,
      profit: 70,
      short_inputs: ['iron'],
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
    { state_id: 1, good_id: 'zany_tools', base: 40, price: 50, buy: 2, sell: 7 },
    { state_id: 2, good_id: 'zany_tools', base: 40, price: 50, buy: 8, sell: 1 },
    { state_id: 3, good_id: 'zany_tools', base: 40, price: 50, buy: 3, sell: 4 },
    { state_id: 4, good_id: 'zany_tools', base: 40, price: 50, buy: 5, sell: 6 },
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
        result={{ ...result, goods: [{ ...result.goods[0], price: result.goods[0].base }] }}
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
            { id: 'cheap', name: 'Cheap', base: 10, price: 17, buy: 1, sell: 1 },
            { id: 'dear', name: 'Dear', base: 200, price: 220, buy: 1, sell: 1 },
          ],
        }}
      />,
    )

    await user.click(screen.getByRole('button', { name: 'Sort by % from base price' }))
    await user.click(screen.getByRole('button', { name: 'Sort by % from base price' }))
    const body = screen.getAllByRole('rowgroup')[1]
    expect(within(body).getAllByRole('row')[0]).toHaveTextContent('Cheap')
  })

  it('links a good to sortable state orders, then a state to building economics', async () => {
    const user = userEvent.setup()
    render(<PriceExplorer result={result} />)

    await user.click(screen.getByRole('link', { name: 'Zany Tools' }))
    expect(await screen.findByRole('heading', { name: 'Zany Tools by state' })).toBeInTheDocument()
    expect(screen.getByText(/not a MAPI local price/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Sort by State' })).toBeInTheDocument()

    await user.click(screen.getByRole('link', { name: 'Alpaca' }))
    expect(
      await screen.findByRole('heading', { name: 'Alpaca buildings' }),
    ).toBeInTheDocument()
    expect(screen.getByText('Silly Hammer Factory')).toBeInTheDocument()
    expect(screen.getByText('70.00')).toBeInTheDocument()
    expect(screen.getByText('Iron')).toBeInTheDocument()
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

  it('switches between domestic and market scope for states and buildings', async () => {
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
    expect(await screen.findByText('Zebra Mill')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Our market' }))
    expect(screen.queryByText('Zebra Mill')).not.toBeInTheDocument()
    expect(screen.getByText('No modeled buildings in this state.')).toBeInTheDocument()
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
            goods_with_orders: 0,
          },
        }}
      />,
    )

    const warning = screen.getByRole('status')
    expect(warning).toHaveTextContent('No buy or sell orders were reconstructed')
    expect(warning).toHaveTextContent(
      '41,234 pops in the save were missing size_wa/size_dn (or legacy size) or wealth',
    )
    expect(warning).toHaveTextContent('none of the 12 buildings use a production method')
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
            goods_with_orders: 2,
          },
        }}
      />,
    )

    expect(screen.queryByRole('status')).not.toBeInTheDocument()
  })
})
