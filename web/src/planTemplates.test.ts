import { describe, expect, it } from 'vitest'
import { PLAN_TEMPLATES, planTemplate } from './planTemplates'

describe('plan templates', () => {
  it('provides unique defaults for every requested planning objective', () => {
    expect(new Set(PLAN_TEMPLATES.map(({ id }) => id)).size).toBe(PLAN_TEMPLATES.length)
    expect(PLAN_TEMPLATES.map(({ id }) => id)).toEqual([
      'war-readiness',
      'colonize',
      'military-size',
      'economic-growth',
      'maximize-revenue',
      'avoid-default',
      'standard-of-living',
    ])
  })

  it('uses goals understood by the current DSL', () => {
    expect(planTemplate('war-readiness')?.goal).toBe('declare-war(state=alsace)')
    expect(planTemplate('war-readiness')?.goal).not.toMatch(/tag=|wargoal=/)
    expect(planTemplate('colonize')?.goal).toBe('colonize(region=region_congo)')
    expect(planTemplate('military-size')?.goal).toBe('army_power_projection >= 100')
    expect(planTemplate('economic-growth')?.goal).toBe('gdp >= 100000000')
    expect(planTemplate('avoid-default')?.goal).toBe('credit_headroom > 0')
    expect(planTemplate('maximize-revenue')?.goal).toBe('weekly_balance >= 100')
    expect(planTemplate('standard-of-living')?.goal).toBe(
      'population_weighted_wealth >= 20',
    )
  })

  it('marks only closable goals as timeline-capable', () => {
    expect(planTemplate('economic-growth')?.closesTimeline).toBe(true)
    expect(planTemplate('military-size')?.closesTimeline).toBe(true)
    expect(planTemplate('avoid-default')?.closesTimeline).toBe(true)
    expect(planTemplate('war-readiness')?.closesTimeline).toBe(false)
    expect(planTemplate('colonize')?.closesTimeline).toBe(false)
    expect(planTemplate('maximize-revenue')?.closesTimeline).toBe(false)
    expect(planTemplate('standard-of-living')?.closesTimeline).toBe(false)
  })
})
