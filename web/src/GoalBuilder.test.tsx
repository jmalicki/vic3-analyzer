import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { GoalBuilder } from './GoalBuilder'

describe('GoalBuilder', () => {
  it('builds guided GDP and goods-price goals', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    const { rerender } = render(
      <GoalBuilder
        idPrefix="Plan"
        goods={['grain', 'wood']}
        value="research(tech=nitroglycerin)"
        onChange={onChange}
      />,
    )

    await user.selectOptions(screen.getByLabelText('Plan goal type'), 'gdp')
    await user.clear(screen.getByLabelText('Plan target GDP'))
    await user.type(screen.getByLabelText('Plan target GDP'), '250000000')
    expect(onChange).toHaveBeenLastCalledWith('gdp >= 250000000')

    rerender(
      <GoalBuilder
        idPrefix="Plan"
        goods={['grain', 'wood']}
        value="gdp >= 250000000"
        onChange={onChange}
      />,
    )
    await user.selectOptions(screen.getByLabelText('Plan goal type'), 'good-price')
    await user.selectOptions(screen.getByLabelText('Plan good'), 'wood')
    await user.selectOptions(screen.getByLabelText('Plan comparison'), '>=')
    await user.clear(screen.getByLabelText('Plan price'))
    await user.type(screen.getByLabelText('Plan price'), '25')
    expect(onChange).toHaveBeenLastCalledWith('good_price(wood) >= 25')
  })

  it('builds declare-war readiness without ignored tag or wargoal args', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(
      <GoalBuilder
        idPrefix="Gaps"
        goods={[]}
        value=""
        onChange={onChange}
        initialKind="declare-war"
      />,
    )

    expect(onChange).toHaveBeenCalledWith('declare-war(state=alsace)')
    await user.clear(screen.getByLabelText('Gaps target state'))
    await user.type(screen.getByLabelText('Gaps target state'), 'bavaria')
    expect(onChange).toHaveBeenLastCalledWith('declare-war(state=bavaria)')
    expect(screen.queryByLabelText('Gaps target country')).not.toBeInTheDocument()
    expect(onChange.mock.calls.flat().join('\n')).not.toMatch(/tag=|wargoal=/)
  })
})
