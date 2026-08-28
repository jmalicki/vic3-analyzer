import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { AlertsPane, LocalRecommendations, hrefForAlert } from './AlertsPane'
import type { AlertsResult, BuildingEconomics } from './types'

const rye: BuildingEconomics = {
  id: 9,
  state_id: 1,
  building_type_name: 'building_rye_farm',
  level: 4,
  staffing: 3,
  inputs: [],
  outputs: [{ name: 'grain', quantity: 10, value: 200 }],
  revenue: 200,
  cost: 50,
  profit: 150,
  short_inputs: [],
}

const alpaca = { id: 1, region_name: 'STATE_ALPACA', label: 'Alpaca' }
const zebra = { id: 2, region_name: 'STATE_ZEBRA', label: 'Zebra' }

const result: AlertsResult = {
  alerts: [
    {
      id: 'electricity_shortage:electricity',
      kind: 'electricity_shortage',
      severity: 1,
      title: 'Electricity shortage',
      summary: 'Buy 40 vs sell 5.',
      good_name: 'electricity',
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
          action: { type: 'build', building_type_name: 'building_rye_farm', extra_levels: 1 },
          effect:
            '~+10 grain sell, covering 25% of the 40 gap. Assumes the new level is staffed at current productivity.',
        },
        {
          id: 'goods:electricity:trade',
          title: 'Reallocate trade',
          detail: 'Trade stays frozen.',
          rank: 3,
          apply_ready: false,
          action: { type: 'trade_alloc', state_id: 1, good_name: 'electricity' },
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
  it('hides foreign-state alerts when playerCountryId is set', () => {
    const multi: AlertsResult = {
      alerts: [
        {
          id: 'home',
          kind: 'unfilled_education',
          severity: 1,
          title: 'Home needs more Farmers',
          summary: 'Player state.',
          state_id: 1,
          evidence: [],
          mitigations: [],
        },
        {
          id: 'rival',
          kind: 'unfilled_education',
          severity: 1,
          title: 'Rivalia needs more Farmers',
          summary: 'Foreign state.',
          state_id: 2,
          evidence: [],
          mitigations: [],
        },
        {
          id: 'global',
          kind: 'goods_shortage',
          severity: 1,
          title: 'mock_lumber shortage',
          summary: 'Null state_id stays visible.',
          evidence: [],
          mitigations: [],
        },
      ],
      limitations: [],
    }
    const home = { id: 1, region_name: 'STATE_HOME', label: 'Home', country_id: 10 }
    const rival = { id: 2, region_name: 'STATE_RIVALIA', label: 'Rivalia', country_id: 99 }
    render(
      <AlertsPane
        result={multi}
        states={[home, rival]}
        playerCountryId={10}
      />,
    )
    expect(screen.getByText('Home needs more Farmers')).toBeInTheDocument()
    expect(screen.getByText('mock_lumber shortage')).toBeInTheDocument()
    expect(screen.queryByText('Rivalia needs more Farmers')).not.toBeInTheDocument()
  })

  it('lists grouped links without embedding Apply on the index', () => {
    render(<AlertsPane result={result} states={[alpaca]} buildings={[rye]} />)

    expect(screen.getByRole('link', { name: /Electricity shortage/ })).toHaveAttribute(
      'href',
      '#/prices/good/electricity',
    )
    expect(screen.getByRole('link', { name: 'Alpaca' })).toHaveAttribute('href', '#/states/1')
    expect(screen.queryByRole('link', { name: 'Building' })).not.toBeInTheDocument()
    expect(document.querySelector('details.alert-expander')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Apply' })).not.toBeInTheDocument()
  })

  it('names unique states instead of repeating Building for each mitigation', () => {
    const building = (id: number, stateId: number): BuildingEconomics => ({
      ...rye,
      id,
      state_id: stateId,
    })
    render(
      <AlertsPane
        result={{
          alerts: [
            {
              id: 'goods_shortage:artillery',
              kind: 'goods_shortage',
              severity: 1,
              title: 'Artillery shortage',
              summary: 'Buy exceeds sell.',
              good_name: 'artillery',
              evidence: [],
              mitigations: [
                {
                  id: 'tc-1',
                  title: 'Subsidize trade center 1',
                  detail: 'Unprofitable.',
                  rank: 1,
                  apply_ready: false,
                  action: { type: 'subsidize', building_id: 1 },
                },
                {
                  id: 'tc-2',
                  title: 'Subsidize trade center 2',
                  detail: 'Unprofitable.',
                  rank: 2,
                  apply_ready: false,
                  action: { type: 'subsidize', building_id: 2 },
                },
                {
                  id: 'tc-3',
                  title: 'Subsidize trade center 3',
                  detail: 'Unprofitable.',
                  rank: 3,
                  apply_ready: false,
                  action: { type: 'subsidize', building_id: 3 },
                },
              ],
            },
          ],
          limitations: [],
        }}
        states={[alpaca, zebra]}
        buildings={[building(1, 1), building(2, 1), building(3, 2)]}
      />,
    )
    expect(screen.getByRole('link', { name: 'Alpaca' })).toHaveAttribute('href', '#/states/1')
    expect(screen.getByRole('link', { name: 'Zebra' })).toHaveAttribute('href', '#/states/2')
    expect(screen.queryByRole('link', { name: 'Building' })).not.toBeInTheDocument()
    expect(screen.queryByRole('link', { name: 'State' })).not.toBeInTheDocument()
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
              good_name: 'grain',
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
              id: 'underemployed:1',
              kind: 'underemployed',
              severity: 2,
              title: 'Test buildings cannot fill Machinists jobs',
              summary: 'Test is short 40 Machinists. 1 building below full staffing.',
              state_id: 1,
              evidence: [],
              mitigations: [],
              staffing: [
                {
                  building_id: 3,
                  building_type_label: 'Textile Mills',
                  building_type_name: 'building_textile_mill',
                  staffing: 6,
                  level: 8,
                  professions: [
                    {
                      name: 'machinists',
                      label: 'Machinists',
                      employed_here: 200,
                      jobs_here: 240,
                      missing_here: 40,
                      state_jobs: 240,
                      state_stock: 200,
                      state_shortage: 40,
                    },
                  ],
                },
              ],
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
    expect(screen.getByRole('link', { name: /Test buildings cannot fill Machinists/ })).toHaveAttribute(
      'href',
      '#/states/1',
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
    expect(screen.getByText('Test buildings cannot fill Machinists jobs')).toBeInTheDocument()
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

  it('nests collapsible buildings under a state employment alert', async () => {
    const user = userEvent.setup()
    render(
      <LocalRecommendations
        alerts={[
          {
            id: 'underemployed:1',
            kind: 'underemployed',
            severity: 2,
            title: 'Test buildings cannot fill Machinists jobs',
            summary: 'Test is short 90 Machinists. Extra levels add more empty jobs.',
            state_id: 1,
            evidence: [],
            mitigations: [
              {
                id: 'under:1:qual',
                title: 'See the Machinists qualification shortage for Test',
                detail:
                  'These buildings are waiting on qualified workers who do not exist in this state. The steps that create those qualifications are listed on that shortage, not on each mill.',
                rank: 1,
                apply_ready: false,
              },
            ],
            staffing: [
              {
                building_id: 4,
                building_type_label: 'Textile Mills',
                building_type_name: 'building_textile_mill',
                staffing: 6,
                level: 8,
                professions: [
                  {
                    name: 'laborers',
                    label: 'Laborers',
                    employed_here: 400,
                    jobs_here: 400,
                    missing_here: 0,
                    state_jobs: 400,
                    state_stock: 500,
                    state_shortage: 0,
                  },
                  {
                    name: 'machinists',
                    label: 'Machinists',
                    employed_here: 200,
                    jobs_here: 240,
                    missing_here: 40,
                    state_jobs: 340,
                    state_stock: 250,
                    state_shortage: 90,
                  },
                ],
              },
              {
                building_id: 5,
                building_type_label: 'Tooling Workshops',
                building_type_name: 'building_tooling_workshops',
                staffing: 1,
                level: 2,
                professions: [
                  {
                    name: 'machinists',
                    label: 'Machinists',
                    employed_here: 50,
                    jobs_here: 100,
                    missing_here: 50,
                    state_jobs: 340,
                    state_stock: 250,
                    state_shortage: 90,
                  },
                ],
              },
            ],
          },
        ]}
      />,
    )

    await user.click(screen.getByText('Test buildings cannot fill Machinists jobs'))
    expect(screen.getAllByRole('link', { name: 'Open building' })[0]).toHaveAttribute(
      'href',
      '#/buildings/building/4',
    )
    expect(screen.getByText(/40 more Machinists/)).toBeInTheDocument()
    expect(screen.getAllByText(/this is blocking/).length).toBeGreaterThanOrEqual(1)
    expect(
      screen.getByText(/See the Machinists qualification shortage for Test/),
    ).toBeInTheDocument()
    expect(
      screen.getByText(/listed on that shortage, not on each mill/),
    ).toBeInTheDocument()

    const mill = document.querySelector('.alert-staffing details')
    expect(mill).toHaveAttribute('open')
    await user.click(mill!.querySelector('span')!)
    expect(mill).not.toHaveAttribute('open')
  })
})
