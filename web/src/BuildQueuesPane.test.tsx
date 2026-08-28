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
      country_id: 1,
      state_id: 1,
      building_type_name: 'building_logging_camp',
      building_type_label: 'Logging Camp',
      state_label: 'Brandenburg',
      remaining: 5,
    },
  ],
  government: [
    {
      id: 1,
      queue: 'government',
      country_id: 1,
      state_id: 2,
      building_type_name: 'building_construction_sector',
      building_type_label: 'Construction Sector',
      state_label: 'Silesia',
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
