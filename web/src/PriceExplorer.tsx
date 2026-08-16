import { useEffect, useMemo, useState } from 'react'
import type {
  BuildingEconomics,
  BuildingGroupInfo,
  BuildingTypeInfo,
  CountryInfo,
  GoodPrice,
  MarketInputs,
  PricesResult,
  StateGood,
  StateInfo,
  StatePop,
} from './types'

type Direction = 'asc' | 'desc'
type SortState<K extends string> = { key: K; direction: Direction }
type View = { kind: 'goods' } | { kind: 'good'; id: string } | { kind: 'state'; id: number }
type FilterMode = 'our_market' | 'domestic' | 'all'

function currentView(): View {
  const path = window.location.hash.replace(/^#\/?/, '').split('/')
  if (path[0] !== 'prices') return { kind: 'goods' }
  if (path[1] === 'good' && path[2]) return { kind: 'good', id: decodeURIComponent(path[2]) }
  if (path[1] === 'state' && Number.isFinite(Number(path[2]))) {
    return { kind: 'state', id: Number(path[2]) }
  }
  return { kind: 'goods' }
}

function displayId(id: string): string {
  return id
    .replace(/^STATE_/, '')
    .replace(/^(building|pm)_/, '')
    .split('_')
    .map((word) => {
      const lower = word.toLowerCase()
      return lower.charAt(0).toUpperCase() + lower.slice(1)
    })
    .join(' ')
}

function goodName(good: GoodPrice): string {
  return good.name || displayId(good.id)
}

/** Good id → PNG data URL, absent when the blob was built without game icons. */
type Icons = Record<string, string>

function GoodIcon({ id, icons }: { id: string; icons: Icons }) {
  const src = icons[id]
  if (!src) return null
  return <img className="good-icon" src={src} alt="" width={24} height={24} />
}

/**
 * Vic3 moves a price by a fraction of its base, so the percentage is the
 * comparable figure across goods; the currency amount is the tooltip.
 */
function PriceDelta({ price, base }: { price: number; base: number }) {
  const delta = price - base
  const percent = base > 0 ? (delta / base) * 100 : 0
  const amount = `${delta > 0 ? '+' : delta < 0 ? '−' : ''}${Math.abs(delta).toFixed(2)} vs base price ${base.toFixed(2)}`
  if (Math.abs(percent) < 0.05) {
    return (
      <span className="delta delta-flat" title={amount}>
        at base price
      </span>
    )
  }
  const up = delta > 0
  return (
    <span className={`delta ${up ? 'delta-up' : 'delta-down'}`} title={amount}>
      <span aria-hidden="true">{up ? '▲' : '▼'}</span>
      {`${up ? '+' : '−'}${Math.abs(percent).toFixed(1)}%`}
    </span>
  )
}

function percentFromBase(price: number, base: number): number {
  return base > 0 ? (price - base) / base : 0
}

function sortRows<T, K extends string>(
  rows: T[],
  sort: SortState<K>,
  value: (row: T, key: K) => string | number,
): T[] {
  const direction = sort.direction === 'asc' ? 1 : -1
  return [...rows].sort((left, right) => {
    const a = value(left, sort.key)
    const b = value(right, sort.key)
    return (typeof a === 'string' && typeof b === 'string'
      ? a.localeCompare(b)
      : Number(a) - Number(b)) * direction
  })
}

function SortButton<K extends string>({
  label,
  sortKey,
  sort,
  onSort,
}: {
  label: string
  sortKey: K
  sort: SortState<K>
  onSort: (key: K) => void
}) {
  const active = sort.key === sortKey
  return (
    <button
      type="button"
      className="table-sort"
      onClick={() => onSort(sortKey)}
      aria-label={`Sort by ${label}`}
    >
      {label} {active ? (sort.direction === 'asc' ? '▲' : '▼') : ''}
    </button>
  )
}

function useSort<K extends string>(initial: K, direction: Direction = 'asc') {
  const [sort, setSort] = useState<SortState<K>>({ key: initial, direction })
  const onSort = (key: K) =>
    setSort((current) => ({
      key,
      direction: current.key === key && current.direction === 'asc' ? 'desc' : 'asc',
    }))
  return [sort, onSort] as const
}

function ScopeFilter({
  mode,
  onChange,
}: {
  mode: FilterMode
  onChange: (mode: FilterMode) => void
}) {
  const options: Array<{ mode: FilterMode; label: string }> = [
    { mode: 'our_market', label: 'Our market' },
    { mode: 'domestic', label: 'Domestic' },
    { mode: 'all', label: 'All' },
  ]
  return (
    <div className="scope-filter" role="group" aria-label="Attributed data scope">
      {options.map((option) => (
        <button
          key={option.mode}
          type="button"
          aria-pressed={mode === option.mode}
          onClick={() => onChange(option.mode)}
        >
          {option.label}
        </button>
      ))}
    </div>
  )
}

type GoodSort = 'name' | 'price' | 'delta' | 'buy' | 'sell'

function GoodsTable({ goods, icons }: { goods: GoodPrice[]; icons: Icons }) {
  const [sort, onSort] = useSort<GoodSort>('name')
  const sorted = useMemo(
    () =>
      sortRows(goods, sort, (good, key) => {
        if (key === 'name') return goodName(good)
        if (key === 'delta') return percentFromBase(good.price, good.base)
        return good[key]
      }),
    [goods, sort],
  )
  return (
    <div className="table-scroll">
      <table>
        <thead>
          <tr>
            <th><SortButton label="Good" sortKey="name" sort={sort} onSort={onSort} /></th>
            <th>Base price</th>
            <th><SortButton label="Price" sortKey="price" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="% from base price" sortKey="delta" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="Buy" sortKey="buy" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="Sell" sortKey="sell" sort={sort} onSort={onSort} /></th>
          </tr>
        </thead>
        <tbody>
          {sorted.map((good) => (
            <tr key={good.id}>
              <th>
                <a
                  className="good-link"
                  href={`#/prices/good/${encodeURIComponent(good.id)}`}
                  title={good.id}
                >
                  <GoodIcon id={good.id} icons={icons} />
                  {goodName(good)}
                </a>
              </th>
              <td>{good.base.toFixed(2)}</td>
              <td>{good.price.toFixed(2)}</td>
              <td><PriceDelta price={good.price} base={good.base} /></td>
              <td>{good.buy.toFixed(2)}</td>
              <td>{good.sell.toFixed(2)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

type StateSort = 'name' | 'price' | 'delta' | 'buy' | 'sell'
type StateRow = StateGood & { state?: StateInfo }

function CountryFlag({
  countryId,
  playerCountryId,
  countries,
  showPlayer = false,
}: {
  countryId?: number
  playerCountryId?: number
  countries: CountryInfo[]
  showPlayer?: boolean
}) {
  if (countryId == null || (!showPlayer && (playerCountryId == null || countryId === playerCountryId))) {
    return null
  }
  const country = countries.find((row) => row.id === countryId)
  if (!country) return null
  const title = country.name || country.tag
  if (country.flag_data_url) {
    return (
      <img
        className="country-flag"
        src={country.flag_data_url}
        alt=""
        title={title}
        width={28}
        height={18}
      />
    )
  }
  return (
    <span className="country-tag" title={title}>
      {country.tag}
    </span>
  )
}

function StatesTable({
  rows,
  countries,
  playerCountryId,
}: {
  rows: StateRow[]
  countries: CountryInfo[]
  playerCountryId?: number
}) {
  const [sort, onSort] = useSort<StateSort>('name')
  const sorted = useMemo(
    () =>
      sortRows(rows, sort, (row, key) => {
        if (key === 'name') return displayId(row.state?.region_id || `State ${row.state_id}`)
        if (key === 'delta') return percentFromBase(row.price, row.base)
        return row[key]
      }),
    [rows, sort],
  )
  return (
    <div className="table-scroll">
      <table>
        <thead>
          <tr>
            <th><SortButton label="State" sortKey="name" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="Market price" sortKey="price" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="% from base price" sortKey="delta" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="State buy" sortKey="buy" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="State sell" sortKey="sell" sort={sort} onSort={onSort} /></th>
          </tr>
        </thead>
        <tbody>
          {sorted.map((row) => (
            <tr key={row.state_id}>
              <th>
                <a className="state-link" href={`#/prices/state/${row.state_id}`}>
                  <CountryFlag
                    countryId={row.state?.country_id}
                    playerCountryId={playerCountryId}
                    countries={countries}
                  />
                  {displayId(row.state?.region_id || `State ${row.state_id}`)}
                </a>
              </th>
              <td>{row.price.toFixed(2)}</td>
              <td><PriceDelta price={row.price} base={row.base} /></td>
              <td>{row.buy.toFixed(2)}</td>
              <td>{row.sell.toFixed(2)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

const STATE_CATEGORIES = ['urban', 'rural', 'development', 'military', 'infrastructure', 'other']

function inheritedGroupValue(
  group: BuildingGroupInfo | undefined,
  groups: Map<string, BuildingGroupInfo>,
  key: 'category' | 'land_usage',
): string | undefined {
  const seen = new Set<string>()
  let current = group
  while (current && !seen.has(current.id)) {
    seen.add(current.id)
    if (current[key]) return current[key]
    current = current.parent_group ? groups.get(current.parent_group) : undefined
  }
  return undefined
}

function flowText(building: BuildingEconomics, side: 'inputs' | 'outputs'): string {
  return building[side]
    .map((flow) => `${displayId(flow.good_id)} ${flow.quantity.toFixed(1)} (${flow.value.toFixed(2)})`)
    .join(', ') || '—'
}

type StateBuildingRow =
  | { kind: 'building'; building: BuildingEconomics; type?: BuildingTypeInfo; group?: BuildingGroupInfo }
  | { kind: 'empty-rural'; capacity: number }
  | { kind: 'constructable'; type: BuildingTypeInfo; group: BuildingGroupInfo }

function StateBuildings({
  state,
  buildings,
  buildingTypes,
  buildingGroups,
}: {
  state?: StateInfo
  buildings: BuildingEconomics[]
  buildingTypes: BuildingTypeInfo[]
  buildingGroups: BuildingGroupInfo[]
}) {
  const types = new Map(buildingTypes.map((building) => [building.id, building]))
  const groups = new Map(buildingGroups.map((group) => [group.id, group]))
  const rows = new Map<string, StateBuildingRow[]>()
  const add = (category: string | undefined, row: StateBuildingRow) => {
    const normalized = category && STATE_CATEGORIES.includes(category.toLowerCase())
      ? category.toLowerCase()
      : 'other'
    rows.set(normalized, [...(rows.get(normalized) ?? []), row])
  }

  for (const building of buildings) {
    const type = types.get(building.type_id)
    const group = type?.group_id ? groups.get(type.group_id) : undefined
    add(inheritedGroupValue(group, groups, 'category'), { kind: 'building', building, type, group })
  }

  const ruralLevels = buildings.reduce((total, building) => {
    const type = types.get(building.type_id)
    const group = type?.group_id ? groups.get(type.group_id) : undefined
    return inheritedGroupValue(group, groups, 'land_usage') === 'rural'
      ? total + building.level
      : total
  }, 0)
  const emptyRural = Math.max(0, (state?.arable_land ?? 0) - ruralLevels)
  if (emptyRural > 0) add('rural', { kind: 'empty-rural', capacity: emptyRural })

  for (const group of buildingGroups) {
    if (!group.always_possible || !group.default_building) continue
    const hasGroup = buildings.some((building) => types.get(building.type_id)?.group_id === group.id)
    const type = types.get(group.default_building)
    if (!hasGroup && type) {
      add(inheritedGroupValue(group, groups, 'category'), { kind: 'constructable', type, group })
    }
  }

  if (![...rows.values()].some((categoryRows) => categoryRows.length)) {
    return <p>No buildings or known capacity in this state.</p>
  }

  return (
    <div className="state-building-groups">
      {STATE_CATEGORIES.filter((category) => rows.has(category)).map((category) => (
        <section className="state-building-group" aria-labelledby={`building-group-${category}`} key={category}>
          <h3 id={`building-group-${category}`}>{displayId(category)}</h3>
          <div className="state-building-list">
            {rows.get(category)!.map((row, index) => {
              if (row.kind === 'empty-rural') {
                return (
                  <article className="state-building-card empty-slot" key="empty-rural">
                    <div><strong>Empty rural land</strong><span>Available capacity</span></div>
                    <b>{row.capacity.toLocaleString()} empty rural slots</b>
                  </article>
                )
              }
              if (row.kind === 'constructable') {
                return (
                  <article className="state-building-card empty-slot" key={`empty-${row.group.id}`}>
                    <div>
                      <strong>{row.type.name || displayId(row.type.id)}</strong>
                      <span>{row.group.name || displayId(row.group.id)}</span>
                    </div>
                    <b>0 levels · constructable placeholder</b>
                  </article>
                )
              }
              const { building, type, group } = row
              const employment = building.level > 0
                ? Math.max(0, Math.min(1, building.staffing / building.level))
                : 0
              return (
                <article className="state-building-card" key={`${building.id}-${index}`}>
                  <div className="state-building-title">
                    <div>
                      <strong>{type?.name || displayId(building.type_id)}</strong>
                      <span>{group?.name || (group ? displayId(group.id) : 'Other')}</span>
                    </div>
                    <b>{building.level.toLocaleString()} levels</b>
                  </div>
                  <dl className="state-building-stats">
                    <div><dt>Employment</dt><dd>{(employment * 100).toFixed(0)}%</dd></div>
                    <div><dt>Model profit</dt><dd>{building.profit.toFixed(2)}</dd></div>
                    <div><dt>Inputs</dt><dd>{flowText(building, 'inputs')}</dd></div>
                    <div><dt>Outputs</dt><dd>{flowText(building, 'outputs')}</dd></div>
                    <div>
                      <dt>Production methods</dt>
                      <dd>{(building.production_method_ids ?? []).map(displayId).join(', ') || '—'}</dd>
                    </div>
                  </dl>
                </article>
              )
            })}
          </div>
        </section>
      ))}
    </div>
  )
}

type PopSort = 'profession' | 'size' | 'wealth' | 'culture'

function StatePops({ pops }: { pops: StatePop[] }) {
  const [sort, onSort] = useSort<PopSort>('size', 'desc')
  const sorted = useMemo(
    () => sortRows(pops, sort, (pop, key) => {
      if (key === 'profession') return pop.profession_name || displayId(pop.profession_id || 'unknown')
      if (key === 'culture') return pop.culture_name || displayId(pop.culture_id || 'unknown')
      if (key === 'size') return pop.demand_size ?? 0
      return pop.wealth ?? 0
    }),
    [pops, sort],
  )
  return (
    <div className="table-scroll">
      <table>
        <thead><tr>
          <th><SortButton label="Profession" sortKey="profession" sort={sort} onSort={onSort} /></th>
          <th><SortButton label="Demand size" sortKey="size" sort={sort} onSort={onSort} /></th>
          <th><SortButton label="Wealth" sortKey="wealth" sort={sort} onSort={onSort} /></th>
          <th><SortButton label="Culture" sortKey="culture" sort={sort} onSort={onSort} /></th>
        </tr></thead>
        <tbody>
          {sorted.map((pop, index) => (
            <tr key={`${pop.profession_id}-${pop.culture_id}-${index}`}>
              <th>{pop.profession_name || displayId(pop.profession_id || 'unknown')}</th>
              <td>{pop.demand_size?.toLocaleString() ?? '—'}</td>
              <td>{pop.wealth ?? '—'}</td>
              <td>{pop.culture_name || displayId(pop.culture_id || 'unknown')}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {!pops.length && <p>No pops in this state.</p>}
    </div>
  )
}

/**
 * A market with no orders prices every good at its base price and still reports
 * `converged`, which is indistinguishable from a balanced economy unless we say
 * so. Name the input that came up empty.
 */
function EmptyMarketWarning({ inputs }: { inputs?: MarketInputs }) {
  if (!inputs || inputs.goods_with_orders > 0) return null
  const causes: string[] = []
  if (inputs.pops === 0) {
    causes.push(
      inputs.skipped_pops > 0
        ? `all ${inputs.skipped_pops.toLocaleString()} pops in the save were missing workforce/dependents (or legacy population fields) or wealth`
        : 'the save has no pops',
    )
  }
  if (inputs.buildings === 0) {
    causes.push('no buildings were read from the save')
  } else if (inputs.buildings_without_orders === inputs.buildings) {
    causes.push(
      `none of the ${inputs.buildings.toLocaleString()} buildings had saved goods IO or a usable production-method fallback`,
    )
  }
  return (
    <p className="model-warning" role="status">
      <strong>No buy or sell orders were reconstructed,</strong> so every good below sits exactly at
      its base price.
      {causes.length ? ` This is because ${causes.join(', and ')}.` : ''}
    </p>
  )
}

export function PriceExplorer({
  result,
  icons = {},
  scenario = false,
  playerCountryId,
  playerMarketId,
}: {
  result: PricesResult
  icons?: Icons
  scenario?: boolean
  playerCountryId?: number
  playerMarketId?: number
}) {
  const [view, setView] = useState<View>(() => currentView())
  const [filterMode, setFilterMode] = useState<FilterMode>('our_market')
  const [stateTab, setStateTab] = useState<'buildings' | 'pops'>('buildings')
  useEffect(() => {
    const update = () => {
      const next = currentView()
      setView(next)
      if (next.kind === 'state') setStateTab('buildings')
    }
    window.addEventListener('hashchange', update)
    return () => window.removeEventListener('hashchange', update)
  }, [])

  const states = result.states ?? []
  const stateGoods = result.state_goods ?? []
  const buildings = result.buildings ?? []
  const buildingTypes = result.building_types ?? []
  const buildingGroups = result.building_groups ?? []
  const statePops = result.state_pops ?? []
  const countries = result.countries ?? []
  const missingPlayerMarket = filterMode === 'our_market' && playerMarketId == null
  const effectiveFilterMode: FilterMode = missingPlayerMarket ? 'all' : filterMode
  const stateIsInScope = (state?: StateInfo) => {
    if (effectiveFilterMode === 'all') return true
    if (effectiveFilterMode === 'our_market') return state?.market_id === playerMarketId
    return state?.country_id === playerCountryId
  }

  if (view.kind === 'good') {
    const good = result.goods.find((row) => row.id === view.id)
    const rows = stateGoods
      .filter((row) => row.good_id === view.id)
      .map((row) => ({ ...row, state: states.find((state) => state.id === row.state_id) }))
      .filter((row) => stateIsInScope(row.state))
    return (
      <section aria-labelledby="good-state-heading">
        <nav className="breadcrumbs" aria-label="Price detail">
          <a href="#/prices">Goods</a><span>›</span><span>{good ? goodName(good) : displayId(view.id)}</span>
        </nav>
        <div className="result-heading">
          <h2 id="good-state-heading">
            <GoodIcon id={view.id} icons={icons} />
            {good ? goodName(good) : displayId(view.id)} by state
          </h2>
          <span>{rows.length} states</span>
        </div>
        <ScopeFilter mode={effectiveFilterMode} onChange={setFilterMode} />
        {missingPlayerMarket && (
          <p className="model-info">Player market unavailable; showing all states.</p>
        )}
        <p className="model-info">
          The shared price is whole-save synthetic, not a MAPI local price. This filter scopes only
          the locally attributed state buy/sell orders.
        </p>
        {rows.length ? (
          <StatesTable rows={rows} countries={countries} playerCountryId={playerCountryId} />
        ) : (
          <p>No state-attributed orders for this good.</p>
        )}
      </section>
    )
  }

  if (view.kind === 'state') {
    const state = states.find((row) => row.id === view.id)
    const rows = buildings.filter((building) => building.state_id === view.id)
    const pops = statePops.filter((pop) => pop.state_id === view.id)
    const name = state?.region_name || displayId(state?.region_id || `State ${view.id}`)
    const owner = countries.find((country) => country.id === state?.country_id)
    return (
      <section aria-labelledby="state-heading" className="state-panel">
        <nav className="breadcrumbs" aria-label="Price detail">
          <a href="#/prices">Goods</a><span>›</span><span>{name}</span>
        </nav>
        <div className="state-header">
          <div className="state-owner">
            <CountryFlag
              countryId={state?.country_id}
              playerCountryId={playerCountryId}
              countries={countries}
              showPlayer
            />
            <div>
              <h2 id="state-heading">{name}</h2>
              <span>{owner?.name || owner?.tag || 'Owner unavailable'}</span>
            </div>
          </div>
          <dl className="state-capacity">
            <div><dt>Arable land</dt><dd>{state?.arable_land?.toLocaleString() ?? '—'}</dd></div>
            {state?.infrastructure != null && (
              <div>
                <dt>Infrastructure</dt>
                <dd>
                  {state.infrastructure.toLocaleString()}
                  {state.infrastructure_usage != null
                    ? ` (${state.infrastructure_usage.toLocaleString()} used)`
                    : ''}
                </dd>
              </div>
            )}
          </dl>
        </div>
        <div className="state-tabs" role="tablist" aria-label="State details">
          <button
            type="button"
            role="tab"
            aria-selected={stateTab === 'buildings'}
            onClick={() => setStateTab('buildings')}
          >
            Buildings
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={stateTab === 'pops'}
            onClick={() => setStateTab('pops')}
          >
            Pops
          </button>
        </div>
        {stateTab === 'buildings' ? (
          <div role="tabpanel" aria-label="Buildings">
            <p className="model-info">
              Profit and goods flows are estimates at whole-save synthetic prices. Empty rows show
              saved rural capacity or broadly available defaults, not full construction eligibility.
            </p>
            <StateBuildings
              state={state}
              buildings={rows}
              buildingTypes={buildingTypes}
              buildingGroups={buildingGroups}
            />
          </div>
        ) : (
          <div role="tabpanel" aria-label="Pops">
            <StatePops pops={pops} />
          </div>
        )}
      </section>
    )
  }

  return (
    <section aria-labelledby="prices-heading">
      <div className="result-heading">
        <h2 id="prices-heading">{scenario ? 'Scenario prices' : 'Goods prices'}</h2>
        <span>{result.goods.length} goods</span>
      </div>
      <p className="model-info">Scope: whole-save synthetic market.</p>
      <EmptyMarketWarning inputs={result.inputs} />
      <GoodsTable goods={result.goods} icons={icons} />
    </section>
  )
}
