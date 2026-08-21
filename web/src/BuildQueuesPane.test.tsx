import { cleanup, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it } from 'vitest'
import { BuildQueuesPane } from './BuildQueuesPane'
import type { ConstructionsSnapshot } from './types'

afterEach(cleanup)

const snapshot: ConstructionsSnapshot = {
  private: [
    {
      id: 10,
      queue: 'private',
      building: 'building_logging_camp',
      building_name: 'Logging Camp',
      state_name: 'Brandenburg',
      remaining: 5,
    },
  ],
  government: [
    {
      id: 1,
      queue: 'government',
      building: 'building_construction_sector',
      building_name: 'Construction Sector',
      state_name: 'Silesia',
      remaining: 40,
    },
  ],
}

function queueTabs() {
  return within(screen.getByRole('tablist', { name: 'Construction queue' }))
}

describe('BuildQueuesPane', () => {
  it('shows government queue by default and switches to private', async () => {
    const user = userEvent.setup()
    render(<BuildQueuesPane snapshot={snapshot} />)
    expect(screen.getByText('Construction Sector')).toBeInTheDocument()
    expect(screen.queryByText('Logging Camp')).not.toBeInTheDocument()

    await user.click(queueTabs().getByRole('tab', { name: /^Private$/ }))
    expect(screen.getByText('Logging Camp')).toBeInTheDocument()
    expect(screen.queryByText('Construction Sector')).not.toBeInTheDocument()
  })

  it('shows empty copy when the selected queue has no orders', async () => {
    const user = userEvent.setup()
    render(<BuildQueuesPane snapshot={{ private: [], government: [] }} />)
    expect(screen.getByText('No government constructions in the queue.')).toBeInTheDocument()
    await user.click(queueTabs().getByRole('tab', { name: /^Private$/ }))
    expect(screen.getByText('No private constructions in the queue.')).toBeInTheDocument()
  })
})
