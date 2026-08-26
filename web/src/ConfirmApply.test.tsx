import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  ConfirmApply,
  deltaForMitigation,
  worldDeltaToSavePatch,
} from './ConfirmApply'
import type { BuildingEconomics, PricesResult, WorldDelta } from './types'

const current: PricesResult = {
  goods: [
    { good_name: 'iron', good_label: 'Iron', base: 40, price: 43.5, buy: 120, sell: 100 },
    { good_name: 'grain', good_label: 'Grain', base: 20, price: 18, buy: 4, sell: 8 },
  ],
  residual: 0.02,
  status: 'converged',
  limitations: [],
}

const preview: PricesResult = {
  goods: [
    { good_name: 'iron', good_label: 'Iron', base: 40, price: 41, buy: 120, sell: 110 },
    { good_name: 'grain', good_label: 'Grain', base: 20, price: 19.25, buy: 4, sell: 8 },
  ],
  residual: 0.001,
  status: 'converged',
  limitations: [],
}

const delta: WorldDelta = {
  extra_levels: [{ building: 'building_rye_farm', extra_levels: 2 }],
}

const rye: BuildingEconomics = {
  id: 9,
  state_id: 1,
  type_id: 'building_rye_farm',
  level: 4,
  staffing: 3,
  production_method_ids: ['pm_simple_farming'],
  inputs: [],
  outputs: [{ good_name: 'grain', quantity: 10, value: 200 }],
  revenue: 200,
  cost: 50,
  profit: 150,
  short_inputs: [],
}

afterEach(cleanup)

describe('ConfirmApply', () => {
  it('renders before/after residual and prices, and Confirm calls onConfirm', async () => {
    const user = userEvent.setup()
    const onConfirm = vi.fn()
    render(
      <ConfirmApply
        delta={delta}
        current={current}
        preview={preview}
        onConfirm={onConfirm}
        onCancel={() => {}}
      />,
    )

    expect(screen.getByRole('dialog', { name: 'Confirm apply' })).toBeInTheDocument()
    expect(screen.getByText('+2 levels on building rye farm')).toBeInTheDocument()
    expect(screen.getByText('0.02')).toBeInTheDocument()
    expect(screen.getByText('0.001')).toBeInTheDocument()
    expect(screen.getByText('43.50')).toBeInTheDocument()
    expect(screen.getByText('41.00')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Confirm' }))
    expect(onConfirm).toHaveBeenCalledTimes(1)
  })

  it('maps alert build/pm/feeder actions to extra levels or production methods', () => {
    expect(
      deltaForMitigation({ type: 'build', building: 'building_rye_farm', extra_levels: 1 }, [rye]),
    ).toEqual({ extra_levels: [{ building_id: 9, extra_levels: 1 }] })
    expect(
      deltaForMitigation(
        {
          type: 'pm',
          building_id: 9,
          production_method: 'pm_automatic',
          methods: ['pm_soil_enriching_farming', 'pm_automatic'],
        },
        [rye],
      ),
    ).toEqual({
      production_methods: [
        { building_id: 9, methods: ['pm_soil_enriching_farming', 'pm_automatic'] },
      ],
    })
    expect(
      deltaForMitigation(
        { type: 'feeder_job', building: 'building_rye_farm', profession: 'farmers' },
        [rye],
      ),
    ).toEqual({ extra_levels: [{ building_id: 9, extra_levels: 1 }] })
    expect(
      deltaForMitigation({ type: 'sol_goods', good_name: 'grain', state_id: 1 }, [rye]),
    ).toEqual({ extra_levels: [{ building_id: 9, extra_levels: 1 }] })
    expect(deltaForMitigation({ type: 'trade_alloc', state_id: 1, good_name: 'grain' }, [rye])).toBeUndefined()
    expect(deltaForMitigation({ type: 'build', building: 'building_missing' }, [rye])).toBeUndefined()
  })

  it('expands type-wide extra levels into a SavePatch of building ids', () => {
    expect(worldDeltaToSavePatch(delta, [rye])).toEqual({
      extra_levels: [{ building_id: 9, extra_levels: 2 }],
      production_methods: [],
    })
  })
})
