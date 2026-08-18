import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { MilitaryPane } from './MilitaryPane'
import type { MilitarySnapshot } from './types'

const snapshot: MilitarySnapshot = {
  armies: [
    {
      id: 1,
      name: 'Armée du Nord',
      type: 'army',
      organization: 85,
      current_manpower: 12000,
      units: [{ id: 11, name: '1st Infantry', type: 'line_infantry', manpower: 1000 }],
    },
  ],
  navies: [
    {
      id: 2,
      name: 'Atlantic Fleet',
      type: 'navy',
      current_manpower: 4000,
      units: [{ id: 21, name: 'HMS Vic', type: 'man_o_war' }],
    },
  ],
  mobilization: [{ id: 3, name: 'General Mobilization', type: 'general' }],
  limitations: ['Headquarters HQ names are not listed yet.'],
}

const empty: MilitarySnapshot = {
  armies: [],
  navies: [],
  mobilization: [],
  limitations: ['Military IR incomplete; missing managers yield empty lists'],
}

afterEach(() => {
  cleanup()
})

describe('MilitaryPane', () => {
  it('renders army names, manpower, and units from the snapshot', () => {
    render(<MilitaryPane snapshot={snapshot} tab="army" />)

    expect(screen.getByText('Armée du Nord')).toBeInTheDocument()
    expect(screen.getByText(/12,000 manpower/)).toBeInTheDocument()
    expect(screen.getByText(/85 organization/)).toBeInTheDocument()
    expect(screen.getByText('1st Infantry')).toBeInTheDocument()
    expect(screen.queryByText('Atlantic Fleet')).not.toBeInTheDocument()
    expect(screen.getByText('Headquarters HQ names are not listed yet.')).toBeInTheDocument()
  })

  it('says lists are empty and shows limitations instead of zero counts', () => {
    render(<MilitaryPane snapshot={empty} tab="army" />)

    expect(screen.getByText('No armies recorded in this save.')).toBeInTheDocument()
    expect(
      screen.getByText('Military IR incomplete; missing managers yield empty lists'),
    ).toBeInTheDocument()
    expect(screen.queryByText(/0 armies/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/^0$/)).not.toBeInTheDocument()
  })

  it('lists navy formations on the navy tab', () => {
    render(<MilitaryPane snapshot={snapshot} tab="navy" />)

    expect(screen.getByText('Atlantic Fleet')).toBeInTheDocument()
    expect(screen.getByText('HMS Vic')).toBeInTheDocument()
    expect(screen.queryByText('Armée du Nord')).not.toBeInTheDocument()
  })

  it('lists mobilization entries or none recorded', () => {
    const { rerender } = render(<MilitaryPane snapshot={snapshot} tab="mobilization" />)
    expect(screen.getByText('General Mobilization')).toBeInTheDocument()

    rerender(<MilitaryPane snapshot={empty} tab="mobilization" />)
    expect(screen.getByText('None recorded')).toBeInTheDocument()
    expect(
      screen.getByText('Military IR incomplete; missing managers yield empty lists'),
    ).toBeInTheDocument()
  })
})
