import { describe, expect, it } from 'vitest'
import { PLAN_TEMPLATES, planTemplate } from './planTemplates'

describe('plan templates', () => {
  it('provides unique defaults for every requested planning objective', () => {
    expect(new Set(PLAN_TEMPLATES.map(({ id }) => id)).size).toBe(PLAN_TEMPLATES.length)
    expect(PLAN_TEMPLATES.map(({ id }) => id)).toEqual([
      'war-readiness',
      'military-size',
      'economic-growth',
      'maximize-revenue',
      'avoid-default',
      'standard-of-living',
    ])
  })

  it('uses goals understood by the current DSL', () => {
    expect(planTemplate('war-readiness')?.goal).toContain('declare-war')
    expect(planTemplate('military-size')?.goal).toBe('army_power_projection >= 100')
    expect(planTemplate('economic-growth')?.goal).toBe('gdp >= 100000000')
    expect(planTemplate('avoid-default')?.goal).toBe('credit_headroom > 0')
    expect(planTemplate('maximize-revenue')?.goal).toBe('weekly_balance >= 100')
    expect(planTemplate('standard-of-living')?.goal).toBe(
      'population_weighted_wealth >= 20',
    )
  })
})
