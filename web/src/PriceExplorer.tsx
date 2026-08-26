import { Fragment, useEffect, useMemo, useState } from 'react'
import {
  LocalRecommendations,
  alertsForBuilding,
  alertsForGood,
  alertsForState,
} from './AlertsPane'
import { GameIcon } from './GameIcon'
import type {
  Alert,
  BuildingEconomics,
  BuildingGroupInfo,
  BuildingTypeInfo,
  CountryInfo,
  DefsIcons,
  GoodFlow,
  GoodPrice,
  MarketInputs,
  PopNeedBasket,
  PricesResult,
  ProfessionCount,
  StateGood,
  StateInfo,
  StateNeed,
  StatePop,
  StateQualification,
  WorldDelta,
} from './types'

type Direction = 'asc' | 'desc'
type SortState<K extends string> = { key: K; direction: Direction }
type View =
  | { kind: 'goods' }
  | { kind: 'good'; id: string }
  | { kind: 'state'; id: number; from: 'prices' | 'states' }
  | { kind: 'building'; id: number; from: 'prices' | 'buildings' }
  | { kind: 'states' }
export type FilterMode = 'our_market' | 'domestic' | 'all'
type StateTab = 'overview' | 'buildings' | 'population' | 'prices' | 'information'

export function currentView(): View {
  const path = window.location.hash.replace(/^#\/?/, '').split('/')
  if (path[0] === 'states') {
    if (path[1] && Number.isFinite(Number(path[1]))) {
      return { kind: 'state', id: Number(path[1]), from: 'states' }
    }
    return { kind: 'states' }
  }
  if (path[0] === 'buildings' && path[1] === 'building' && Number.isFinite(Number(path[2]))) {
    return { kind: 'building', id: Number(path[2]), from: 'buildings' }
  }
  if (path[0] !== 'prices') return { kind: 'goods' }
  if (path[1] === 'good' && path[2]) return { kind: 'good', id: decodeURIComponent(path[2]) }
  if (path[1] === 'state' && Number.isFinite(Number(path[2]))) {
    return { kind: 'state', id: Number(path[2]), from: 'prices' }
  }
  if (path[1] === 'building' && Number.isFinite(Number(path[2]))) {
    return { kind: 'building', id: Number(path[2]), from: 'prices' }
  }
  return { kind: 'goods' }
}

export function displayId(id: string): string {
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
  return good.good_label || displayId(good.good_name)
}

/** Game icons from a defs blob; goods may be nested or a flat id → URL map. */
type Icons = DefsIcons

function GoodIcon({ id, icons }: { id: string; icons: Icons }) {
  return <GameIcon kind="good" id={id} icons={icons} />
}

function GoodFlows({
  flows,
  goods,
  icons,
}: {
  flows: GoodFlow[]
  goods: GoodPrice[]
  icons: Icons
}) {
  if (!flows.length) return <>—</>
  return (
    <ul className="good-chips">
      {flows.map((flow) => {
        const good = goods.find((row) => row.good_name === flow.good_name)
        return (
          <li key={flow.good_name}>
            <a className="good-link" href={`#/prices/good/${encodeURIComponent(flow.good_name)}`}>
              <GoodIcon id={flow.good_name} icons={icons} />
              {good ? goodName(good) : displayId(flow.good_name)}
              {' '}
              {flow.quantity.toFixed(1)} ({flow.value.toFixed(2)})
            </a>
          </li>
        )
      })}
    </ul>
  )
}

export function NeedBaskets({
  needs,
  goods,
  icons,
}: {
  needs: PopNeedBasket[]
  goods: GoodPrice[]
  icons: Icons
}) {
  if (!needs.length) return <p>No modeled needs for this selection.</p>
  return (
    <div className="need-baskets">
      {needs.map((need) => (
        <article key={need.need_id} className="need-basket">
          <h4>{need.need_name || displayId(need.need_id)}</h4>
          {need.goods.length ? (
            <table>
              <thead>
                <tr>
                  <th>Good</th>
                  <th>Amount</th>
                </tr>
              </thead>
              <tbody>
                {need.goods.map((flow) => {
                  const good = goods.find((row) => row.good_name === flow.good_name)
                  return (
                    <tr key={flow.good_name}>
                      <th>
                        <a
                          className="good-link"
                          href={`#/prices/good/${encodeURIComponent(flow.good_name)}`}
                        >
                          <GoodIcon id={flow.good_name} icons={icons} />
                          {good ? goodName(good) : displayId(flow.good_name)}
                        </a>
                      </th>
                      <td>{flow.quantity.toFixed(1)}</td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          ) : (
            <p>No goods in this basket.</p>
          )}
        </article>
      ))}
    </div>
  )
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

export function sortRows<T, K extends string>(
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

export function SortButton<K extends string>({
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

export function useSort<K extends string>(initial: K, direction: Direction = 'asc') {
  const [sort, setSort] = useState<SortState<K>>({ key: initial, direction })
  const onSort = (key: K) =>
    setSort((current) => ({
      key,
      direction: current.key === key && current.direction === 'asc' ? 'desc' : 'asc',
    }))
  return [sort, onSort] as const
}

export function ScopeFilter({
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

function aggregateGoods(
  goods: GoodPrice[],
  stateGoods: StateGood[],
  states: StateInfo[],
  stateIsInScope: (state?: StateInfo) => boolean,
): GoodPrice[] {
  const statesById = new Map(states.map((state) => [state.id, state]))
  const scopedRows = stateGoods.filter((row) => stateIsInScope(statesById.get(row.state_id)))
  const rowsByGood = new Map<string, StateGood[]>()
  for (const row of scopedRows) {
    const rows = rowsByGood.get(row.good_name)
    if (rows) rows.push(row)
    else rowsByGood.set(row.good_name, [row])
  }
  return goods.map((good) => {
    const rows = rowsByGood.get(good.good_name)
    if (!rows?.length) return good
    let buy = 0
    let sell = 0
    let weightedPrice = 0
    let weight = 0
    for (const row of rows) {
      buy += row.buy
      sell += row.sell
      const rowWeight = Math.max(0, row.buy) + Math.max(0, row.sell)
      weightedPrice += row.price * rowWeight
      weight += rowWeight
    }
    return {
      ...good,
      buy,
      sell,
      price: weight > 0 ? weightedPrice / weight : good.base,
    }
  })
}

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
            <tr key={good.good_name}>
              <th>
                <a
                  className="good-link"
                  href={`#/prices/good/${encodeURIComponent(good.good_name)}`}
                  title={good.good_name}
                >
                  <GoodIcon id={good.good_name} icons={icons} />
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

export function CountryFlag({
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
            <th><SortButton label="State price" sortKey="price" sort={sort} onSort={onSort} /></th>
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

type StateBuildingRow =
  | { kind: 'building'; building: BuildingEconomics; type?: BuildingTypeInfo; group?: BuildingGroupInfo }
  | { kind: 'empty-rural'; capacity: number }
  | { kind: 'constructable'; type: BuildingTypeInfo; group: BuildingGroupInfo }

function StateBuildings({
  state,
  buildings,
  buildingTypes,
  buildingGroups,
  goods,
  icons,
}: {
  state?: StateInfo
  buildings: BuildingEconomics[]
  buildingTypes: BuildingTypeInfo[]
  buildingGroups: BuildingGroupInfo[]
  goods: GoodPrice[]
  icons: Icons
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
                      <strong>
                        <GameIcon kind="building" id={row.type.id} icons={icons} />
                        {row.type.name || displayId(row.type.id)}
                      </strong>
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
              const name = type?.name || displayId(building.type_id)
              return (
                <article className="state-building-card" key={`${building.id}-${index}`}>
                  <div className="state-building-title">
                    <div>
                      <a className="building-link" href={`#/prices/building/${building.id}`}>
                        <GameIcon kind="building" id={building.type_id} icons={icons} />
                        <strong>{name}</strong>
                      </a>
                      <span>{group?.name || (group ? displayId(group.id) : 'Other')}</span>
                    </div>
                    <b>{building.level.toLocaleString()} levels</b>
                  </div>
                  <dl className="state-building-stats">
                    <div><dt>Employment</dt><dd>{(employment * 100).toFixed(0)}%</dd></div>
                    <div><dt>Revenue</dt><dd>{building.revenue.toFixed(2)}</dd></div>
                    <div><dt>Cost</dt><dd>{building.cost.toFixed(2)}</dd></div>
                    <div><dt>Model profit</dt><dd>{building.profit.toFixed(2)}</dd></div>
                    <div><dt>Inputs</dt><dd><GoodFlows flows={building.inputs} goods={goods} icons={icons} /></dd></div>
                    <div><dt>Outputs</dt><dd><GoodFlows flows={building.outputs} goods={goods} icons={icons} /></dd></div>
                    {building.short_inputs.length > 0 && (
                      <div>
                        <dt>Short inputs</dt>
                        <dd>{building.short_inputs.map(displayId).join(', ')}</dd>
                      </div>
                    )}
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

type PopSort = 'profession' | 'workforce' | 'dependents' | 'wealth' | 'culture'

function StatePops({
  pops,
  qualifications,
  stateNeeds,
  goods,
  icons,
}: {
  pops: StatePop[]
  qualifications: StateQualification[]
  stateNeeds: StateNeed[]
  goods: GoodPrice[]
  icons: Icons
}) {
  const [sort, onSort] = useSort<PopSort>('workforce', 'desc')
  const [open, setOpen] = useState<number | null>(null)
  const sorted = useMemo(
    () => sortRows(pops, sort, (pop, key) => {
      if (key === 'profession') return pop.profession_name || displayId(pop.profession_id || 'unknown')
      if (key === 'culture') return pop.culture_name || displayId(pop.culture_id || 'unknown')
      if (key === 'workforce') return pop.workforce ?? pop.demand_size ?? 0
      if (key === 'dependents') return pop.dependents ?? 0
      return pop.wealth ?? 0
    }),
    [pops, sort],
  )
  return (
    <div className="state-population">
      <p className="model-info">
        Needs are model baskets at solved prices (package ladder + substitution), not a save cashflow ledger.
      </p>
      {stateNeeds.length > 0 && (
        <section className="state-needs-strip" aria-label="State needs">
          <h3>State needs</h3>
          <NeedBaskets
            needs={stateNeeds.map((need) => ({
              need_id: need.need_id,
              need_name: need.need_name,
              package_value: need.package_value,
              goods: need.goods,
            }))}
            goods={goods}
            icons={icons}
          />
        </section>
      )}
      {qualifications.length > 0 && (
        <section aria-label="Qualifications">
          <h3>Qualifications</h3>
          <p className="model-info">
            Shortage is filled jobs minus employable (or qualified) stock. Monthly qualification gain is omitted unless the save stores it.
          </p>
          <QualificationsTable rows={qualifications} />
        </section>
      )}
      <div className="table-scroll">
        <table>
          <thead><tr>
            <th><SortButton label="Profession" sortKey="profession" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="Workforce" sortKey="workforce" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="Dependents" sortKey="dependents" sort={sort} onSort={onSort} /></th>
            <th>Literacy</th>
            <th><SortButton label="Wealth" sortKey="wealth" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="Culture" sortKey="culture" sort={sort} onSort={onSort} /></th>
          </tr></thead>
          <tbody>
            {sorted.map((pop, index) => {
              const literacy = pop.workforce && pop.workforce > 0 && pop.literate != null
                ? `${((pop.literate / pop.workforce) * 100).toFixed(0)}%`
                : '—'
              return (
                <Fragment key={`${pop.profession_id}-${pop.culture_id}-${index}`}>
                  <tr>
                    <th>
                      <button
                        type="button"
                        className="pop-expand"
                        aria-expanded={open === index}
                        onClick={() => setOpen(open === index ? null : index)}
                      >
                        {pop.profession_id ? (
                          <GameIcon kind="pop" id={pop.profession_id} icons={icons} />
                        ) : null}
                        {pop.profession_name || displayId(pop.profession_id || 'unknown')}
                      </button>
                    </th>
                    <td>{pop.workforce?.toLocaleString() ?? '—'}</td>
                    <td>{pop.dependents?.toLocaleString() ?? '—'}</td>
                    <td>{literacy}</td>
                    <td>{pop.wealth ?? '—'}</td>
                    <td>{pop.culture_name || displayId(pop.culture_id || 'unknown')}</td>
                  </tr>
                  {open === index && (
                    <tr className="pop-detail">
                      <td colSpan={6}>
                        <NeedBaskets needs={pop.needs ?? []} goods={goods} icons={icons} />
                      </td>
                    </tr>
                  )}
                </Fragment>
              )
            })}
          </tbody>
        </table>
        {!pops.length && <p>No pops in this state.</p>}
      </div>
    </div>
  )
}

export function QualificationsTable({ rows }: { rows: StateQualification[] }) {
  return (
    <div className="table-scroll">
      <table>
        <thead>
          <tr>
            <th>Profession</th>
            <th>Employed</th>
            <th>Jobs</th>
            <th>Qualified</th>
            <th>Employable</th>
            <th>Shortage</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.profession_id}>
              <th>{row.profession_name || displayId(row.profession_id)}</th>
              <td>{row.employed.toLocaleString()}</td>
              <td>{row.jobs.toLocaleString()}</td>
              <td>{row.qualified.toLocaleString()}</td>
              <td>{row.employable?.toLocaleString() ?? '—'}</td>
              <td>{row.shortage.toLocaleString()}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

function StateOverview({
  pops,
  buildings,
  qualifications,
  stateNeeds,
  goods,
  icons,
}: {
  pops: StatePop[]
  buildings: BuildingEconomics[]
  qualifications: StateQualification[]
  stateNeeds: StateNeed[]
  goods: GoodPrice[]
  icons: Icons
}) {
  const workforce = pops.reduce((sum, pop) => sum + (pop.workforce ?? 0), 0)
  const dependents = pops.reduce((sum, pop) => sum + (pop.dependents ?? 0), 0)
  const literate = pops.reduce((sum, pop) => sum + (pop.literate ?? 0), 0)
  const literacy = workforce > 0 && pops.some((pop) => pop.literate != null)
    ? `${((literate / workforce) * 100).toFixed(0)}%`
    : '—'
  const levels = buildings.reduce((sum, building) => sum + building.level, 0)
  const profit = buildings.reduce((sum, building) => sum + building.profit, 0)
  const staffing = buildings.reduce((sum, building) => {
    if (building.level <= 0) return sum
    return sum + Math.max(0, Math.min(1, building.staffing / building.level))
  }, 0)
  const avgStaffing = buildings.length ? staffing / buildings.length : 0
  const employed = qualifications.reduce((sum, row) => sum + row.employed, 0)
  const shortage = qualifications.reduce((sum, row) => sum + row.shortage, 0)
  return (
    <div className="state-overview">
      <dl className="overview-kpis">
        <div><dt>Population</dt><dd>{(workforce + dependents).toLocaleString()}</dd></div>
        <div><dt>Workforce</dt><dd>{workforce.toLocaleString()}</dd></div>
        <div><dt>Dependents</dt><dd>{dependents.toLocaleString()}</dd></div>
        <div><dt>Literacy</dt><dd>{literacy}</dd></div>
        <div><dt>Building levels</dt><dd>{levels.toLocaleString()}</dd></div>
        <div><dt>Model profit</dt><dd>{profit.toFixed(2)}</dd></div>
        <div><dt>Avg. employment</dt><dd>{(avgStaffing * 100).toFixed(0)}%</dd></div>
        <div><dt>Employed</dt><dd>{employed.toLocaleString()}</dd></div>
        <div><dt>Jobs shortage</dt><dd>{shortage.toLocaleString()}</dd></div>
      </dl>
      {stateNeeds.length > 0 && (
        <section aria-label="State needs">
          <h3>State needs</h3>
          <NeedBaskets
            needs={stateNeeds.map((need) => ({
              need_id: need.need_id,
              need_name: need.need_name,
              package_value: need.package_value,
              goods: need.goods,
            }))}
            goods={goods}
            icons={icons}
          />
        </section>
      )}
    </div>
  )
}

type LocalPriceSort = 'name' | 'price' | 'buy' | 'sell'

function StateLocalPrices({
  rows,
  goods,
  icons,
}: {
  rows: StateGood[]
  goods: GoodPrice[]
  icons: Icons
}) {
  const [sort, onSort] = useSort<LocalPriceSort>('name')
  const sorted = useMemo(
    () => sortRows(rows, sort, (row, key) => {
      if (key === 'name') {
        const good = goods.find((item) => item.good_name === row.good_name)
        return good ? goodName(good) : displayId(row.good_name)
      }
      return row[key]
    }),
    [rows, sort, goods],
  )
  return (
    <div>
      <p className="model-info">
        Local prices blend the solved market price with each state&apos;s attributed-order price
        using infrastructure-only market access and base MAPI 75%.
      </p>
      {rows.length ? (
        <div className="table-scroll">
          <table>
            <thead>
              <tr>
                <th><SortButton label="Good" sortKey="name" sort={sort} onSort={onSort} /></th>
                <th><SortButton label="Local price" sortKey="price" sort={sort} onSort={onSort} /></th>
                <th>Market price</th>
                <th>State price</th>
                <th><SortButton label="Buy" sortKey="buy" sort={sort} onSort={onSort} /></th>
                <th><SortButton label="Sell" sortKey="sell" sort={sort} onSort={onSort} /></th>
                <th>Market access</th>
              </tr>
            </thead>
            <tbody>
              {sorted.map((row) => {
                const good = goods.find((item) => item.good_name === row.good_name)
                return (
                  <tr key={row.good_name}>
                    <th>
                      <a className="good-link" href={`#/prices/good/${encodeURIComponent(row.good_name)}`}>
                        <GoodIcon id={row.good_name} icons={icons} />
                        {good ? goodName(good) : displayId(row.good_name)}
                      </a>
                    </th>
                    <td>{row.price.toFixed(2)}</td>
                    <td>{row.market_price.toFixed(2)}</td>
                    <td>{row.state_price.toFixed(2)}</td>
                    <td>{row.buy.toFixed(2)}</td>
                    <td>{row.sell.toFixed(2)}</td>
                    <td>{(row.market_access * 100).toFixed(0)}%</td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
      ) : (
        <p>No local prices for this state.</p>
      )}
    </div>
  )
}

function StateInformation({
  state,
  owner,
}: {
  state?: StateInfo
  owner?: CountryInfo
}) {
  return (
    <dl className="state-information">
      <div><dt>Region</dt><dd>{displayId(state?.region_id || '—')}</dd></div>
      <div><dt>Region id</dt><dd>{state?.region_id || '—'}</dd></div>
      <div><dt>Country</dt><dd>{owner?.name || owner?.tag || '—'}</dd></div>
      <div><dt>Market id</dt><dd>{state?.market_id ?? '—'}</dd></div>
      <div><dt>Arable land</dt><dd>{state?.arable_land?.toLocaleString() ?? '—'}</dd></div>
      <div>
        <dt>Infrastructure</dt>
        <dd>
          {state?.infrastructure != null
            ? `${state.infrastructure.toLocaleString()}${
                state.infrastructure_usage != null
                  ? ` (${state.infrastructure_usage.toLocaleString()} used)`
                  : ''
              }`
            : '—'}
        </dd>
      </div>
      <div>
        <dt>Not modeled</dt>
        <dd>State traits, resources, incorporation, and tax capacity are unavailable from the current save IR.</dd>
      </div>
    </dl>
  )
}

export function StatePage({
  result,
  icons = {},
  playerCountryId,
  stateId,
  source = 'prices',
  alerts = [],
  onApply,
}: {
  result: PricesResult
  icons?: DefsIcons
  playerCountryId?: number
  stateId: number
  source?: 'prices' | 'states'
  alerts?: Alert[]
  onApply?: (delta: WorldDelta) => void
}) {
  const [stateTab, setStateTab] = useState<StateTab>('overview')
  useEffect(() => {
    setStateTab('overview')
  }, [stateId])

  const states = result.states ?? []
  const stateGoods = result.state_goods ?? []
  const buildings = result.buildings ?? []
  const buildingTypes = result.building_types ?? []
  const buildingGroups = result.building_groups ?? []
  const statePops = result.state_pops ?? []
  const stateQualifications = result.state_qualifications ?? []
  const stateNeeds = result.state_needs ?? []
  const countries = result.countries ?? []
  const state = states.find((row) => row.id === stateId)
  const rows = buildings.filter((building) => building.state_id === stateId)
  const pops = statePops.filter((pop) => pop.state_id === stateId)
  const qualifications = stateQualifications.filter((row) => row.state_id === stateId)
  const needs = stateNeeds.filter((row) => row.state_id === stateId)
  const localGoods = stateGoods.filter((row) => row.state_id === stateId)
  const name = state?.state_name || displayId(state?.region_id || `State ${stateId}`)
  const owner = countries.find((country) => country.id === state?.country_id)
  const tabs: Array<{ id: StateTab; label: string }> = [
    { id: 'overview', label: 'Overview' },
    { id: 'buildings', label: 'Buildings' },
    { id: 'population', label: 'Population' },
    { id: 'prices', label: 'Local Prices' },
    { id: 'information', label: 'Information' },
  ]
  const parentHref = source === 'states' ? '#/states' : '#/prices'
  const parentLabel = source === 'states' ? 'States' : 'Goods'
  const localAlerts = alertsForState(alerts, stateId, buildings)
  return (
    <section aria-labelledby="state-heading" className="state-panel">
      <nav className="breadcrumbs" aria-label={source === 'states' ? 'State detail' : 'Price detail'}>
        <a href={parentHref}>{parentLabel}</a><span>›</span><span>{name}</span>
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
      </div>
      <LocalRecommendations
        alerts={localAlerts}
        buildings={buildings}
        icons={icons}
        onApply={onApply}
      />
      <div className="state-tabs" role="tablist" aria-label="State details">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            type="button"
            role="tab"
            aria-selected={stateTab === tab.id}
            onClick={() => setStateTab(tab.id)}
          >
            {tab.label}
          </button>
        ))}
      </div>
      {stateTab === 'overview' && (
        <div role="tabpanel" aria-label="Overview">
          <StateOverview
            pops={pops}
            buildings={rows}
            qualifications={qualifications}
            stateNeeds={needs}
            goods={result.goods}
            icons={icons}
          />
        </div>
      )}
      {stateTab === 'buildings' && (
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
            goods={result.goods}
            icons={icons}
          />
        </div>
      )}
      {stateTab === 'population' && (
        <div role="tabpanel" aria-label="Population">
          <StatePops
            pops={pops}
            qualifications={qualifications}
            stateNeeds={needs}
            goods={result.goods}
            icons={icons}
          />
        </div>
      )}
      {stateTab === 'prices' && (
        <div role="tabpanel" aria-label="Local Prices">
          <StateLocalPrices rows={localGoods} goods={result.goods} icons={icons} />
        </div>
      )}
      {stateTab === 'information' && (
        <div role="tabpanel" aria-label="Information">
          <StateInformation state={state} owner={owner} />
        </div>
      )}
    </section>
  )
}

function EmployeesTable({ employees }: { employees: ProfessionCount[] }) {
  if (!employees.length) return <p>No workplace pops linked to this building.</p>
  return (
    <div className="table-scroll">
      <table>
        <thead>
          <tr>
            <th>Profession</th>
            <th>Employed</th>
          </tr>
        </thead>
        <tbody>
          {employees.map((row) => (
            <tr key={row.profession_id}>
              <th>{row.profession_name || displayId(row.profession_id)}</th>
              <td>{row.count.toLocaleString()}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

export function BuildingPage({
  result,
  icons = {},
  buildingId,
  source = 'prices',
  alerts = [],
  onApply,
}: {
  result: PricesResult
  icons?: DefsIcons
  buildingId: number
  source?: 'prices' | 'buildings'
  alerts?: Alert[]
  onApply?: (delta: WorldDelta) => void
}) {
  const states = result.states ?? []
  const buildings = result.buildings ?? []
  const buildingTypes = result.building_types ?? []
  const building = buildings.find((row) => row.id === buildingId)
  const state = states.find((row) => row.id === building?.state_id)
  const type = buildingTypes.find((row) => row.id === building?.type_id)
  const name = type?.name || displayId(building?.type_id || `Building ${buildingId}`)
  const stateName = state?.state_name || displayId(state?.region_id || (state ? `State ${state.id}` : 'State'))
  const employment = building && building.level > 0
    ? Math.max(0, Math.min(1, building.staffing / building.level))
    : 0
  const parentHref = source === 'buildings' ? '#/buildings' : '#/prices'
  const parentLabel = source === 'buildings' ? 'Buildings' : 'Goods'
  const stateHref = state
    ? source === 'buildings'
      ? `#/states/${state.id}`
      : `#/prices/state/${state.id}`
    : undefined
  return (
    <section aria-labelledby="building-heading" className="state-panel">
      <nav className="breadcrumbs" aria-label={source === 'buildings' ? 'Building detail' : 'Price detail'}>
        <a href={parentHref}>{parentLabel}</a><span>›</span>
        {state && stateHref ? <a href={stateHref}>{stateName}</a> : <span>{stateName}</span>}
        <span>›</span><span>{name}</span>
      </nav>
      <div className="state-header">
        <div>
          <h2 id="building-heading">{name}</h2>
          <span>{building ? `${building.level.toLocaleString()} levels` : 'Building unavailable'}</span>
        </div>
      </div>
      {building ? (
        <>
          <dl className="overview-kpis">
            <div><dt>Employment</dt><dd>{(employment * 100).toFixed(0)}%</dd></div>
            <div><dt>Revenue</dt><dd>{building.revenue.toFixed(2)}</dd></div>
            <div><dt>Cost</dt><dd>{building.cost.toFixed(2)}</dd></div>
            <div><dt>Model profit</dt><dd>{building.profit.toFixed(2)}</dd></div>
          </dl>
          <dl className="state-building-stats">
            <div><dt>Inputs</dt><dd><GoodFlows flows={building.inputs} goods={result.goods} icons={icons} /></dd></div>
            <div><dt>Outputs</dt><dd><GoodFlows flows={building.outputs} goods={result.goods} icons={icons} /></dd></div>
          </dl>
          <h3>Workforce</h3>
          <EmployeesTable employees={building.employees ?? []} />
          <LocalRecommendations
            alerts={alertsForBuilding(alerts, building)}
            buildings={buildings}
            icons={icons}
            onApply={onApply}
          />
        </>
      ) : (
        <p>No building with that id.</p>
      )}
    </section>
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
  alerts = [],
  onApply,
}: {
  result: PricesResult
  icons?: Icons
  scenario?: boolean
  playerCountryId?: number
  playerMarketId?: number
  alerts?: Alert[]
  onApply?: (delta: WorldDelta) => void
}) {
  const [view, setView] = useState<View>(() => currentView())
  const [filterMode, setFilterMode] = useState<FilterMode>('our_market')
  useEffect(() => {
    const update = () => setView(currentView())
    window.addEventListener('hashchange', update)
    return () => window.removeEventListener('hashchange', update)
  }, [])

  const states = result.states ?? []
  const stateGoods = result.state_goods ?? []
  const countries = result.countries ?? []
  const missingPlayerMarket = filterMode === 'our_market' && playerMarketId == null
  const effectiveFilterMode: FilterMode = missingPlayerMarket ? 'all' : filterMode
  const stateIsInScope = (state?: StateInfo) => {
    if (effectiveFilterMode === 'all') return true
    if (effectiveFilterMode === 'our_market') return state?.market_id === playerMarketId
    return state?.country_id === playerCountryId
  }
  const scopedGoods = aggregateGoods(result.goods, stateGoods, states, stateIsInScope)

  if (view.kind === 'good') {
    const good = result.goods.find((row) => row.good_name === view.id)
    const rows = stateGoods
      .filter((row) => row.good_name === view.id)
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
          Local prices blend the solved market price with each state&apos;s attributed-order price
          using infrastructure-only market access and base MAPI 75%.
        </p>
        {rows.length ? (
          <StatesTable rows={rows} countries={countries} playerCountryId={playerCountryId} />
        ) : (
          <p>No state-attributed orders for this good.</p>
        )}
        <LocalRecommendations
          alerts={alertsForGood(alerts, view.id)}
          buildings={result.buildings ?? []}
          icons={icons}
          onApply={onApply}
        />
      </section>
    )
  }

  if (view.kind === 'building') {
    return (
      <BuildingPage
        result={result}
        icons={icons}
        buildingId={view.id}
        source={view.from}
        alerts={alerts}
        onApply={onApply}
      />
    )
  }

  if (view.kind === 'state') {
    return (
      <StatePage
        result={result}
        icons={icons}
        playerCountryId={playerCountryId}
        stateId={view.id}
        source={view.from}
        alerts={alerts}
        onApply={onApply}
      />
    )
  }

  if (view.kind === 'states') return null

  const list = (
    <>
      <div className="result-heading">
        {scenario ? <h2 id="prices-heading">Scenario prices</h2> : null}
        <span>{scopedGoods.length} goods</span>
      </div>
      <ScopeFilter mode={effectiveFilterMode} onChange={setFilterMode} />
      {missingPlayerMarket && (
        <p className="model-info">Player market unavailable; showing all states.</p>
      )}
      <p className="model-info">
        Prices are order-weighted averages of state prices in the selected scope.
      </p>
      <EmptyMarketWarning inputs={result.inputs} />
      <GoodsTable goods={scopedGoods} icons={icons} />
    </>
  )
  if (scenario) {
    return <section aria-labelledby="prices-heading">{list}</section>
  }
  return list
}
