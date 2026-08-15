export interface GoodPrice {
  id: string
  base: number
  price: number
  buy: number
  sell: number
}

export interface PricesResult {
  goods: GoodPrice[]
  residual: number
  status: 'converged' | 'max_iters' | 'failed'
  limitations: string[]
}

export type GapAtom = string | Record<string, unknown>

export interface GapsResult {
  satisfied: boolean
  gaps: GapAtom[]
  limitations: string[]
}

export type PlanAction =
  | { QueueTech: { tech: string } }
  | { WaitForEvent: { event: { TechCompleted: { tech: string } }; days: number } }

export interface PlanStep {
  day: number
  action: PlanAction
}

export interface PlanResult {
  day_cost: number
  actions: PlanStep[]
  residual: number
  limitations: string[]
}

export type AnalysisResult = PricesResult | GapsResult | PlanResult

export interface SaveSummary {
  tag?: string
  date?: string
  version: string
  buildings?: string[]
}

export interface DefsSummary {
  goods: number
  production_methods: number
  pop_needs: number
  buy_packages: number
  price_range: number
}

export type AnalysisKind = 'prices' | 'what_if' | 'gaps' | 'plan'

export interface AnalysisRecord {
  id: string
  created_at: string
  label?: string
  kind: AnalysisKind
  fingerprint: string
  date?: string
  country?: string
  filename?: string
  opts: Record<string, unknown>
  result: AnalysisResult
  limitations: string[]
  parent_id?: string
  blob?: {
    save: Uint8Array
    tokens?: Uint8Array
  }
}

export interface ActionDiff {
  left?: PlanStep
  right?: PlanStep
}

export interface PriceDelta {
  good: string
  delta: number
}

export interface GapDiff {
  atom: GapAtom
  status: 'still_failing' | 'cleared' | 'newly_failing'
}

export interface CompareResult {
  left: string
  right: string
  same_fingerprint: boolean
  day_cost_delta?: number
  actions?: ActionDiff[]
  prices?: PriceDelta[]
  gaps?: GapDiff[]
}

export interface JsonSchema {
  title?: string
  description?: string
  type?: string
  format?: string
  default?: unknown
  minimum?: number
  properties?: Record<string, JsonSchema>
  required?: string[]
  $ref?: string
  $defs?: Record<string, JsonSchema>
}
