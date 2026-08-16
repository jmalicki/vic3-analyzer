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
    expect(warning).toHaveTextContent('41,234 pops in the save were missing a size or wealth value')
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
