import type { GoalKind } from './GoalBuilder'

export type PlanTemplateId =
  | 'war-readiness'
  | 'military-size'
  | 'economic-growth'
  | 'maximize-revenue'
  | 'avoid-default'
  | 'standard-of-living'

export interface PlanTemplate {
  id: PlanTemplateId
  title: string
  description: string
  goal?: string
  goalKind: GoalKind
  label: string
}

export const PLAN_TEMPLATES: readonly PlanTemplate[] = [
  {
    id: 'war-readiness',
    title: 'Prepare for war',
    description: 'Check interest, army strength, ammunition prices, and solvency.',
    goal: 'declare-war(tag=FRA, wargoal=conquer_state, state=alsace)',
    goalKind: 'declare-war',
    label: 'War readiness vs FRA',
  },
  {
    id: 'military-size',
    title: 'Build a good-sized military',
    description: 'Target the model’s initial army power threshold.',
    goal: 'army_power_projection >= 100',
    goalKind: 'advanced',
    label: 'Army power 100',
  },
  {
    id: 'economic-growth',
    title: 'Grow the economy',
    description: 'Target a GDP of 100 million.',
    goal: 'gdp >= 100000000',
    goalKind: 'gdp',
    label: 'GDP 100 million',
  },
  {
    id: 'maximize-revenue',
    title: 'Increase weekly income',
    description: 'Target a saved net weekly-budget sample of at least 100.',
    goal: 'weekly_balance >= 100',
    goalKind: 'advanced',
    label: 'Weekly income 100',
  },
  {
    id: 'avoid-default',
    title: 'Avoid default',
    description: 'Require known remaining credit before the debt limit.',
    goal: 'credit_headroom > 0',
    goalKind: 'advanced',
    label: 'Avoid default',
  },
  {
    id: 'standard-of-living',
    title: 'Raise standard of living',
    description: 'Target population-weighted saved pop wealth of at least 20 as an SoL proxy.',
    goal: 'population_weighted_wealth >= 20',
    goalKind: 'advanced',
    label: 'Average SoL 20',
  },
]

export function planTemplate(id: string): PlanTemplate | undefined {
  return PLAN_TEMPLATES.find((template) => template.id === id)
}
