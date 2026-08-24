import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it } from 'vitest'
import { PopsPane } from './PopsPane'
import type { PricesResult } from './types'

const result: PricesResult = {
  goods: [{ id: 'apples', name: 'Apples', base: 20, price: 15, buy: 5, sell: 7 }],
  countries: [
    { id: 10, tag: 'ALP', name: 'Alpacania' },
    { id: 20, tag: 'BDG', name: 'Badgeria' },
  ],
  states: [
    {
      id: 1,
      region_id: 'STATE_ALPACA',
      state_name: 'Alpaca',
      country_id: 10,
      market_id: 1,
    },
    { id: 2, region_id: 'STATE_ZEBRA', state_name: 'Zebra', country_id: 10, market_id: 2 },
    { id: 3, region_id: 'STATE_BADGER', state_name: 'Badger', country_id: 20, market_id: 1 },
  ],
  buildings: [
    {
      id: 7,
      state_id: 1,
      type_id: 'building_silly_hammer_factory',
      level: 3,
      staffing: 2.4,
      inputs: [],
      outputs: [],
      revenue: 150,
      cost: 80,
      profit: 70,
      short_inputs: [],
    },
  ],
  building_types: [
    { id: 'building_silly_hammer_factory', name: 'Silly Hammer Factory' },
  ],
  state_pops: [
    {
      state_id: 1,
      profession_id: 'machinists',
      profession_name: 'Machinists',
      workforce: 8000,
      dependents: 4000,
      literate: 2400,
      wealth: 14,
      culture_id: 'north_german',
      culture_name: 'North German',
      workplace_id: 7,
      needs: [
        {
          need_id: 'popneed_staple_foods',
          need_name: 'Staple foods',
          package_value: 10,
          goods: [{ good_id: 'apples', quantity: 4, value: 60 }],
        },
      ],
    },
    {
      state_id: 3,
      profession_id: 'farmers',
      profession_name: 'Farmers',
      workforce: 2000,
      dependents: 1000,
      wealth: 8,
      culture_id: 'south_german',
      culture_name: 'South German',
    },
  ],
  state_qualifications: [
    {
      state_id: 1,
      profession_id: 'engineers',
      profession_name: 'Engineers',
      qualified: 100,
      employable: 80,
      employed: 50,
      jobs: 120,
      shortage: 70,
    },
  ],
  state_needs: [
    {
      state_id: 1,
      need_id: 'popneed_staple_foods',
      need_name: 'Staple foods',
      package_value: 10,
      goods: [{ good_id: 'apples', quantity: 4, value: 60 }],
    },
  ],
  residual: 0,
  status: 'converged',
  limitations: [],
}

afterEach(cleanup)

describe('PopsPane', () => {
  it('renders professions from mock result', () => {
    render(<PopsPane result={result} playerCountryId={10} playerMarketId={1} />)

    expect(screen.getByRole('heading', { name: 'Pops' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Machinists/ })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Farmers/ })).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'Qualification shortages' })).toBeInTheDocument()
    expect(screen.getByText('Engineers')).toBeInTheDocument()
  })

  it('expands a profession group to show a pop and state link', async () => {
    const user = userEvent.setup()
    render(<PopsPane result={result} playerCountryId={10} playerMarketId={1} />)

    expect(screen.queryByText('North German')).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /Machinists/ }))
    expect(screen.getByText('North German')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Alpaca' })).toHaveAttribute('href', '#/states/1')
    expect(screen.getByRole('link', { name: 'Silly Hammer Factory' })).toHaveAttribute(
      'href',
      '#/buildings/building/7',
    )
  })

  it('hides foreign pops when Domestic is selected', async () => {
    const user = userEvent.setup()
    render(<PopsPane result={result} playerCountryId={10} playerMarketId={1} />)

    await user.click(screen.getByRole('button', { name: 'Domestic' }))
    expect(screen.getByRole('button', { name: /Machinists/ })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /Farmers/ })).not.toBeInTheDocument()
  })

  it('shows local pop recommendations for the current scope', () => {
    render(
      <PopsPane
        result={result}
        playerCountryId={10}
        playerMarketId={1}
        alerts={[
          {
            id: 'needs_unmet:1',
            kind: 'needs_unmet',
            severity: 1,
            title: 'Unmet pop needs in Alpaca',
            summary: 'Baskets exceed local sell.',
            state_id: 1,
            evidence: [],
            mitigations: [],
          },
          {
            id: 'needs_unmet:2',
            kind: 'needs_unmet',
            severity: 1,
            title: 'Unmet pop needs in Zebra',
            summary: 'Out of market.',
            state_id: 2,
            evidence: [],
            mitigations: [],
          },
        ]}
      />,
    )

    expect(screen.getByRole('heading', { name: 'Recommendations' })).toBeInTheDocument()
    expect(screen.getByText('Unmet pop needs in Alpaca')).toBeInTheDocument()
    expect(screen.queryByText('Unmet pop needs in Zebra')).not.toBeInTheDocument()
  })
})
