import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { StatesPane } from './StatesPane'
import type { PricesResult } from './types'

const result: PricesResult = {
  goods: [{ name: 'iron', label: 'Iron', base: 40, price: 50, buy: 2, sell: 1 }],
  countries: [
    { id: 10, country_name: 'ALP', country_label: 'Alpacania' },
    { id: 20, country_name: 'BDG', country_label: 'Badgeria' },
  ],
  states: [
    {
      id: 1,
      region_name: 'STATE_ALPACA',
      state_label: 'Alpaca',
      country_id: 10,
      market_id: 1,
      infrastructure: 22,
      infrastructure_usage: 17,
    },
    { id: 2, region_name: 'STATE_ZEBRA', state_label: 'Zebra', country_id: 10, market_id: 2 },
    { id: 3, region_name: 'STATE_BADGER', state_label: 'Badger', country_id: 20, market_id: 1 },
  ],
  state_pops: [
    {
      state_id: 1,
      profession_name: 'machinists',
      workforce: 8000,
      dependents: 4000,
      wealth: 14,
    },
  ],
  residual: 0,
  status: 'converged',
  limitations: [],
}

describe('StatesPane', () => {
  beforeEach(() => {
    window.location.hash = '#/states'
  })
  afterEach(cleanup)

  it('renders state names from PricesResult.states', () => {
    render(<StatesPane result={result} playerCountryId={10} playerMarketId={1} />)

    expect(screen.getByRole('heading', { name: 'States' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: /Alpaca/ })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: /Badger/ })).toBeInTheDocument()
    expect(screen.queryByRole('link', { name: /Zebra/ })).not.toBeInTheDocument()
  })

  it('clicks a row and drills into the state heading', async () => {
    const user = userEvent.setup()
    render(<StatesPane result={result} playerCountryId={10} playerMarketId={1} />)

    await user.click(screen.getByRole('link', { name: /Alpaca/ }))
    expect(window.location.hash).toBe('#/states/1')
    expect(await screen.findByRole('heading', { name: 'Alpaca' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'States' })).toHaveAttribute('href', '#/states')
  })

  it('hides foreign states when Domestic is selected', async () => {
    const user = userEvent.setup()
    render(<StatesPane result={result} playerCountryId={10} playerMarketId={1} />)

    await user.click(screen.getByRole('button', { name: 'Domestic' }))
    expect(screen.getByRole('link', { name: /Alpaca/ })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: /Zebra/ })).toBeInTheDocument()
    expect(screen.queryByRole('link', { name: /Badger/ })).not.toBeInTheDocument()
  })
})
