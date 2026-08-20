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
  /**
   * When true, A* has successors that can close the goal today.
   * Gaps-only presets stay available under Goal gaps.
   */
  closesTimeline: boolean
}

export const PLAN_TEMPLATES: readonly PlanTemplate[] = [
  {
    id: 'war-readiness',
    title: 'Prepare for war',
    description:
      'Readiness for interest, army strength, ammunition prices, and solvency. Interest/army timelines exist; full declare-war still needs solvent already true (or a fiscal model).',
    goal: 'declare-war(state=alsace)',
    goalKind: 'declare-war',
    label: 'War readiness (alsace)',
    closesTimeline: false,
  },
  {
    id: 'military-size',
    title: 'Build a good-sized military',
    description:
      'Raise army power projection to the model’s declare-war threshold via a fixed-time expansion.',
    goal: 'army_power_projection >= 100',
    goalKind: 'advanced',
    label: 'Army power 100',
    closesTimeline: true,
  },
  {
    id: 'economic-growth',
    title: 'Grow the economy',
    description: 'Target a GDP of 100 million via modeled building-level expansions.',
    goal: 'gdp >= 100000000',
    goalKind: 'gdp',
    label: 'GDP 100 million',
    closesTimeline: true,
  },
  {
    id: 'maximize-revenue',
    title: 'Increase weekly income',
    description:
      'Target a saved net weekly-budget sample of at least 100. Gaps only until a fiscal transition model exists.',
    goal: 'weekly_balance >= 100',
    goalKind: 'advanced',
    label: 'Weekly income 100',
    closesTimeline: false,
  },
  {
    id: 'avoid-default',
    title: 'Avoid default',
    description:
      'Require known remaining credit before the debt limit. Gaps only until a fiscal transition model exists.',
    goal: 'credit_headroom > 0',
    goalKind: 'advanced',
    label: 'Avoid default',
    closesTimeline: false,
  },
  {
    id: 'standard-of-living',
    title: 'Raise standard of living',
    description:
      'Target population-weighted saved pop wealth of at least 20 as an SoL proxy. Gaps only until a wage model exists.',
    goal: 'population_weighted_wealth >= 20',
    goalKind: 'advanced',
    label: 'Average SoL 20',
    closesTimeline: false,
  },
]

export function planTemplate(id: string): PlanTemplate | undefined {
  return PLAN_TEMPLATES.find((template) => template.id === id)
}
