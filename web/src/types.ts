export interface GoodPrice {
  id: string
  name?: string
  base: number
  price: number
  buy: number
  sell: number
}

export interface StateInfo {
  id: number
  region_id?: string
  region_name?: string
  country_id?: number
  market_id?: number
  arable_land?: number
  infrastructure?: number
  infrastructure_usage?: number
}

export interface StatePop {
  state_id: number
  id?: number
  profession_id?: string
  profession_name?: string
  demand_size?: number
  workforce?: number
  dependents?: number
  wealth?: number
  culture_id?: string
  culture_name?: string
  literate?: number
  workplace_id?: number
  qualifications?: ProfessionCount[]
  needs?: PopNeedBasket[]
}

export interface ProfessionCount {
  profession_id: string
  profession_name?: string
  count: number
}

export interface PopNeedBasket {
  need_id: string
  need_name?: string
  package_value: number
  goods: GoodFlow[]
}

export interface StateNeed {
  state_id: number
  need_id: string
  need_name?: string
  package_value: number
  goods: GoodFlow[]
}

export interface StateQualification {
  state_id: number
  profession_id: string
  profession_name?: string
  qualified: number
  employable?: number
  employed: number
  jobs: number
  shortage: number
  monthly_change?: number
}

export interface BuildingTypeInfo {
  id: string
  name?: string
  group_id?: string
  city_type?: string
}

export interface BuildingGroupInfo {
  id: string
  name?: string
  category?: string
  land_usage?: string
  always_possible: boolean
  default_building?: string
  parent_group?: string
}

export interface StateGood {
  state_id: number
  good_id: string
  buy: number
  sell: number
  price: number
  market_price: number
  state_price: number
  market_access: number
  effective_mapi: number
  base: number
}

export interface GoodFlow {
  good_id: string
  quantity: number
  value: number
}

export interface BuildingEconomics {
  id: number
  state_id?: number
  type_id: string
  level: number
  staffing: number
  production_method_ids?: string[]
  inputs: GoodFlow[]
  outputs: GoodFlow[]
  revenue: number
  cost: number
  profit: number
  short_inputs: string[]
  employees?: ProfessionCount[]
}

export interface ProductionMethodFlow {
  good: string
  qty: number
}

export interface ProductionMethodDef {
  id: string
  inputs: ProductionMethodFlow[]
  outputs: ProductionMethodFlow[]
}

export interface MarketInputs {
  pops: number
  skipped_pops: number
  buildings: number
  skipped_buildings: number
  buildings_without_method: number
  buildings_without_orders: number
  goods_with_orders: number
}

export interface CountryInfo {
  id: number
  tag: string
  name?: string
  flag_coa?: string
  flag_data_url?: string
}

export interface PricesResult {
  scope?: 'whole_save_synthetic'
  goods: GoodPrice[]
  countries?: CountryInfo[]
  states?: StateInfo[]
  state_goods?: StateGood[]
  buildings?: BuildingEconomics[]
  building_types?: BuildingTypeInfo[]
  building_groups?: BuildingGroupInfo[]
  state_pops?: StatePop[]
  state_qualifications?: StateQualification[]
  state_needs?: StateNeed[]
  inputs?: MarketInputs
  residual: number
  status: 'converged' | 'max_iters' | 'failed'
  limitations: string[]
  gdp?: number
  sol?: number
  population_weighted_wealth?: number
}

export interface ProductionMethodDelta {
  building_id: number
  methods: string[]
}

export interface ExtraLevelsDelta {
  building?: string
  building_id?: number
  extra_levels: number
}

export interface SubsidizeDelta {
  building_id: number
  enabled: boolean
}

export interface WorldDelta {
  production_methods?: ProductionMethodDelta[]
  extra_levels?: ExtraLevelsDelta[]
  subsidize?: SubsidizeDelta[]
}

export type OptimizeAxis = 'income' | 'productivity' | 'sol'

export interface OptimizeChange {
  building_type: string
  building_id: number
  from: string[]
  to: string[]
}

export interface OptimizeDelta {
  income: number
  productivity: number
  sol: number
  residual: number
}

export interface OptimizePricesSummary {
  residual: number
  status: PricesResult['status']
  income: number
  productivity: number
  sol: number
}

export interface OptimizeResult {
  axis: OptimizeAxis
  changes: OptimizeChange[]
  delta: OptimizeDelta
  prices?: OptimizePricesSummary
  limitations: string[]
  world_delta: WorldDelta
}

/** Surgical save rewrite: extra levels and PMs keyed by building instance id. */
export interface SavePatch {
  production_methods?: ProductionMethodDelta[]
  extra_levels?: Array<{ building_id: number; extra_levels: number }>
}

export type GapAtom = string | Record<string, unknown>

export interface GapsResult {
  satisfied: boolean
  gaps: GapAtom[]
  limitations: string[]
}

export type PlanAction =
  | { QueueTech: { tech: string } }
  | { QueueBuildingLevel: { building: string } }
  | {
      WaitForEvent: {
        event:
          | { TechCompleted: { tech: string } }
          | { BuildingCompleted: { building: string } }
        days: number
      }
    }

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

export type AlertKind =
  | 'electricity_shortage'
  | 'transportation_shortage'
  | 'goods_shortage'
  | 'needs_unmet'
  | 'low_market_access'
  | 'unfilled_education'
  | 'unfilled_pops'
  | 'underemployed'

export interface AlertEvidence {
  label: string
  value: string
}

export type MitigationAction =
  | { type: 'build'; building: string; state_id?: number; extra_levels?: number }
  | { type: 'pm'; building_id: number; production_method: string; methods?: string[] }
  | { type: 'subsidize'; building_id: number }
  | { type: 'trade_alloc'; state_id: number; good_id: string }
  | { type: 'feeder_job'; building: string; profession: string; state_id?: number }
  | { type: 'sol_goods'; good_id: string; state_id?: number }

export interface AlertMitigation {
  id: string
  title: string
  detail: string
  rank: number
  action?: MitigationAction
  apply_ready: boolean
  /** Estimated impact on the short good; omitted outside shortage alerts. */
  effect?: string
}

export interface ProfessionGap {
  profession_id: string
  profession_name?: string
  employed_here: number
  jobs_here: number
  missing_here: number
  state_jobs: number
  state_stock: number
  state_shortage: number
}

export interface BuildingStaffing {
  building_id: number
  building_name: string
  type_id: string
  staffing: number
  level: number
  professions: ProfessionGap[]
}

export interface Alert {
  id: string
  kind: AlertKind
  severity: 1 | 2
  title: string
  summary: string
  state_id?: number
  building_id?: number
  good_id?: string
  evidence: AlertEvidence[]
  mitigations: AlertMitigation[]
  staffing?: BuildingStaffing[]
}

export interface AlertsResult {
  alerts: Alert[]
  limitations: string[]
}

export type AnalysisResult = PricesResult | GapsResult | PlanResult

export interface SaveSummary {
  tag?: string
  country_id?: number
  market_id?: number
  date?: string
  version: string
  buildings?: string[]
}

export interface MilitaryUnitSnapshot {
  id?: number
  name?: string
  type?: string
  manpower?: number
}

export interface MilitaryFormationSnapshot {
  id: number
  name?: string
  type?: string
  country?: number
  organization?: number
  current_manpower?: number
  units: MilitaryUnitSnapshot[]
}

export interface MobilizationSnapshot {
  id: number
  name?: string
  country?: number
  type?: string
}

export interface MilitarySnapshot {
  armies: MilitaryFormationSnapshot[]
  navies: MilitaryFormationSnapshot[]
  mobilization: MobilizationSnapshot[]
  limitations: string[]
}

export interface DefsIcons {
  goods?: Record<string, string>
  extra?: Record<string, string>
  [key: string]: string | Record<string, string> | undefined
}

export type GameIconKind = 'good' | 'building' | 'pm' | 'pop' | 'alert' | 'military'

export interface DefsSummary {
  blob_version: number
  goods: number
  labels: number
  icons: number
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

export interface Origin {
  id: string
  name: string
  bytes?: Uint8Array
  blob?: Blob
  tokens?: Uint8Array | Blob
  tokens_name?: string
  fingerprint?: string
  saved_at: string
}

export interface Timeline {
  id: string
  origin_id: string
  label: string
  created_at: string
}

export interface Step {
  id: string
  timeline_id: string
  parent_step_id?: string | null
  mutations: unknown[]
  summary?: SaveSummary
  prices?: PricesResult
  prices_cache_version?: number
  patched_bytes?: Uint8Array
  created_at: string
  label?: string
}

export interface CurrentPointer {
  id: 'current'
  origin_id: string
  timeline_id: string
  step_id: string
}
