import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { AlertsPane } from './AlertsPane'
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
      evidence: [{ label: 'Buy', value: '40' }],
      mitigations: [
        {
          id: 'goods:electricity:local',
          title: 'Local-only good',
          detail: 'electricity is non-tradeable.',
          rank: 1,
          apply_ready: false,
        },
        {
          id: 'goods:electricity:build',
          title: 'Add rye farm levels',
          detail: 'Produce more grain.',
          rank: 2,
          apply_ready: false,
          action: { type: 'build', building: 'building_rye_farm', extra_levels: 1 },
        },
        {
          id: 'goods:electricity:trade',
          title: 'Reallocate trade',
          detail: 'Trade stays frozen.',
          rank: 3,
          apply_ready: false,
          action: { type: 'trade_alloc', state_id: 1, good_id: 'electricity' },
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
  it('opens an expander and keeps Apply disabled without a mappable action', async () => {
    const user = userEvent.setup()
    render(<AlertsPane result={result} />)

    const expander = document.querySelector('details')
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
    render(<AlertsPane result={result} buildings={[rye]} onApply={onApply} />)

    await user.click(screen.getByText('Electricity shortage'))
    const apply = screen.getAllByRole('button', { name: 'Apply' })
    expect(apply[0]).toBeDisabled()
    expect(apply[2]).toBeDisabled()
    expect(apply[1]).toBeEnabled()
    await user.click(apply[1])
    expect(onApply).toHaveBeenCalledWith({ extra_levels: [{ building_id: 9, extra_levels: 1 }] })
  })
})
