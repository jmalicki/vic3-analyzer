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
      production_method_id: 'pm_goofy_hammers',
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

  it('sorts goods by name, price, and delta from base', async () => {
    const user = userEvent.setup()
    render(<PriceExplorer result={result} />)
    const body = screen.getAllByRole('rowgroup')[1]
    expect(within(body).getAllByRole('row')[0]).toHaveTextContent('Apples')

    await user.click(screen.getByRole('button', { name: 'Sort by Price' }))
    expect(within(body).getAllByRole('row')[0]).toHaveTextContent('Apples')
    await user.click(screen.getByRole('button', { name: 'Sort by Price' }))
    expect(within(body).getAllByRole('row')[0]).toHaveTextContent('Zany Tools')

    await user.click(screen.getByRole('button', { name: 'Sort by Δ from base' }))
    expect(within(body).getAllByRole('row')[0]).toHaveTextContent('Apples')
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
})
