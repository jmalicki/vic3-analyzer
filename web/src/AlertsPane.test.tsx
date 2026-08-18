import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { AlertsPane, LocalRecommendations, hrefForAlert } from './AlertsPane'
import type { AlertsResult, BuildingEconomics } from './types'

const rye: BuildingEconomics = {
  id: 9,
  state_id: 1,
  type_id: 'building_rye_farm',
  level: 4,
  staffing: 3,
  inputs: [],
  outputs: [{ good_id: 'grain', quantity: 10, value: 200 }],
  revenue: 200,
  cost: 50,
  profit: 150,
  short_inputs: [],
}

const result: AlertsResult = {
  alerts: [
    {
      id: 'electricity_shortage:electricity',
      kind: 'electricity_shortage',
      severity: 1,
      title: 'Electricity shortage',
      summary: 'Buy 40 vs sell 5.',
      good_id: 'electricity',
      building_id: 9,
      evidence: [{ label: 'Buy', value: '40' }],
      mitigations: [
        {
          id: 'goods:electricity:local',
          title: 'Local-only good',
          detail: 'electricity is non-tradeable.',
          rank: 1,
          apply_ready: false,
          effect: '0 extra electricity from imports (local-only good).',
        },
        {
          id: 'goods:electricity:build',
          title: 'Add rye farm levels',
          detail: 'Produce more grain.',
          rank: 2,
          apply_ready: false,
          action: { type: 'build', building: 'building_rye_farm', extra_levels: 1 },
          effect:
            '~+10 grain sell, covering 25% of the 40 gap. Assumes the new level is staffed at current productivity.',
        },
        {
          id: 'goods:electricity:trade',
          title: 'Reallocate trade',
          detail: 'Trade stays frozen.',
          rank: 3,
          apply_ready: false,
          action: { type: 'trade_alloc', state_id: 1, good_id: 'electricity' },
          effect: '0 extra electricity in this model (trade volumes are frozen).',
        },
      ],
    },
  ],
  limitations: ['Apply is disabled until the apply track.'],
}

afterEach(() => {
  cleanup()
})

describe('AlertsPane', () => {
  it('lists grouped links without embedding Apply on the index', () => {
    render(<AlertsPane result={result} />)

    expect(screen.getByRole('link', { name: /Electricity shortage/ })).toHaveAttribute(
      'href',
      '#/prices/good/electricity',
    )
    expect(screen.getByRole('link', { name: 'Building' })).toHaveAttribute(
      'href',
      '#/buildings/building/9',
    )
    expect(document.querySelector('details.alert-expander')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Apply' })).not.toBeInTheDocument()
  })

  it('groups alerts by type so a shortage group can collapse', async () => {
    const user = userEvent.setup()
    render(
      <AlertsPane
        result={{
          ...result,
          alerts: [
            ...result.alerts,
            {
              id: 'goods_shortage:grain',
              kind: 'goods_shortage',
              severity: 1,
              title: 'Grain shortage',
              summary: 'Buy exceeds sell.',
              good_id: 'grain',
              evidence: [],
              mitigations: [],
            },
            {
              id: 'needs_unmet:1',
              kind: 'needs_unmet',
              severity: 1,
              title: 'Unmet pop needs in Test',
              summary: 'Baskets exceed local sell.',
              state_id: 1,
              evidence: [],
              mitigations: [],
            },
            {
              id: 'underemployed:3',
              kind: 'underemployed',
              severity: 2,
              title: 'Underemployed Rye Farm in Test (Farmers)',
              summary: 'Staffing/level is 33% on Rye Farm in Test (Farmers).',
              building_id: 3,
              state_id: 1,
              evidence: [],
              mitigations: [],
            },
          ],
        }}
      />,
    )
    expect(screen.getByText('Shortages')).toBeInTheDocument()
    expect(screen.getByText('2 alerts')).toBeInTheDocument()
    expect(screen.getByText('Unmet needs')).toBeInTheDocument()
    expect(screen.getByText('Employment')).toBeInTheDocument()
    expect(hrefForAlert(result.alerts[0])).toBe('#/prices/good/electricity')
    expect(screen.getByRole('link', { name: /Underemployed Rye Farm/ })).toHaveAttribute(
      'href',
      '#/buildings/building/3',
    )
    expect(screen.getByRole('link', { name: /Unmet pop needs/ })).toHaveAttribute(
      'href',
      '#/states/1',
    )

    const shortageGroup = document.querySelector('details.alert-group')
    expect(shortageGroup).toHaveAttribute('open')
    await user.click(shortageGroup!.querySelector('summary')!)
    expect(shortageGroup).not.toHaveAttribute('open')
    expect(screen.getByText('Unmet pop needs in Test')).toBeInTheDocument()
    expect(screen.getByText('Underemployed Rye Farm in Test (Farmers)')).toBeInTheDocument()
  })
})

describe('LocalRecommendations', () => {
  it('opens an expander and keeps Apply disabled without a mappable action', async () => {
    const user = userEvent.setup()
    render(<LocalRecommendations alerts={result.alerts} />)

    const expander = document.querySelector('details.alert-expander')
    expect(expander).not.toHaveAttribute('open')
    await user.click(screen.getByText('Electricity shortage'))
    expect(expander).toHaveAttribute('open')

    const apply = screen.getAllByRole('button', { name: 'Apply' })
    expect(apply[0]).toBeDisabled()
    expect(apply[0]).toHaveAttribute('title', 'Cannot apply this mitigation yet')
  })

  it('calls onApply with extra levels when a build mitigation maps to a building', async () => {
    const user = userEvent.setup()
    const onApply = vi.fn()
    render(<LocalRecommendations alerts={result.alerts} buildings={[rye]} onApply={onApply} />)

    await user.click(screen.getByText('Electricity shortage'))
    const apply = screen.getAllByRole('button', { name: 'Apply' })
    expect(apply[0]).toBeDisabled()
    expect(apply[2]).toBeDisabled()
    expect(apply[1]).toBeEnabled()
    await user.click(apply[1])
    expect(onApply).toHaveBeenCalledWith({ extra_levels: [{ building_id: 9, extra_levels: 1 }] })
  })

  it('shows estimated effect on each shortage intervention', async () => {
    const user = userEvent.setup()
    render(<LocalRecommendations alerts={result.alerts} />)
    await user.click(screen.getByText('Electricity shortage'))
    expect(screen.getByText(/Estimated effect: 0 extra electricity from imports/)).toBeInTheDocument()
    expect(screen.getByText(/Estimated effect: ~\+10 grain sell/)).toBeInTheDocument()
    expect(screen.getByText(/Estimated effect: 0 extra electricity in this model/)).toBeInTheDocument()
  })
})
