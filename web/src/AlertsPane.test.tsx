import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it } from 'vitest'
import { AlertsPane } from './AlertsPane'
import type { AlertsResult } from './types'

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
      ],
    },
  ],
  limitations: ['Apply is disabled until the apply track.'],
}

afterEach(() => {
  cleanup()
})

describe('AlertsPane', () => {
  it('opens an expander and keeps Apply disabled', async () => {
    const user = userEvent.setup()
    render(<AlertsPane result={result} />)

    const expander = document.querySelector('details')
    expect(expander).not.toHaveAttribute('open')
    await user.click(screen.getByText('Electricity shortage'))
    expect(expander).toHaveAttribute('open')

    const apply = screen.getByRole('button', { name: 'Apply' })
    expect(apply).toBeDisabled()
    expect(apply).toHaveAttribute('title', 'coming in apply track')
  })
})
