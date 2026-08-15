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

export type AnalysisResult = PricesResult | GapsResult

export interface SaveSummary {
  tag?: string
  date?: string
  version: string
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
