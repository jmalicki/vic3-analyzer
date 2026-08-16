import { useEffect, useMemo, useState } from 'react'
import type { BuildingEconomics, GoodPrice, PricesResult, StateGood, StateInfo } from './types'

type Direction = 'asc' | 'desc'
type SortState<K extends string> = { key: K; direction: Direction }
type View = { kind: 'goods' } | { kind: 'good'; id: string } | { kind: 'state'; id: number }

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

type GoodSort = 'name' | 'price' | 'delta' | 'buy' | 'sell'

function GoodsTable({ goods }: { goods: GoodPrice[] }) {
  const [sort, onSort] = useSort<GoodSort>('name')
  const sorted = useMemo(
    () =>
      sortRows(goods, sort, (good, key) => {
        if (key === 'name') return goodName(good)
        if (key === 'delta') return good.price - good.base
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
            <th>Base</th>
            <th><SortButton label="Price" sortKey="price" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="Δ from base" sortKey="delta" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="Buy" sortKey="buy" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="Sell" sortKey="sell" sort={sort} onSort={onSort} /></th>
          </tr>
        </thead>
        <tbody>
          {sorted.map((good) => (
            <tr key={good.id}>
              <th>
                <a href={`#/prices/good/${encodeURIComponent(good.id)}`} title={good.id}>
                  {goodName(good)}
                </a>
              </th>
              <td>{good.base.toFixed(2)}</td>
              <td>{good.price.toFixed(2)}</td>
              <td>{(good.price - good.base).toFixed(2)}</td>
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

function StatesTable({ rows }: { rows: StateRow[] }) {
  const [sort, onSort] = useSort<StateSort>('name')
  const sorted = useMemo(
    () =>
      sortRows(rows, sort, (row, key) => {
        if (key === 'name') return displayId(row.state?.region_id || `State ${row.state_id}`)
        if (key === 'delta') return row.price - row.base
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
            <th><SortButton label="Δ from base" sortKey="delta" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="State buy" sortKey="buy" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="State sell" sortKey="sell" sort={sort} onSort={onSort} /></th>
          </tr>
        </thead>
        <tbody>
          {sorted.map((row) => (
            <tr key={row.state_id}>
              <th>
                <a href={`#/prices/state/${row.state_id}`}>
                  {displayId(row.state?.region_id || `State ${row.state_id}`)}
                </a>
              </th>
              <td>{row.price.toFixed(2)}</td>
              <td>{(row.price - row.base).toFixed(2)}</td>
              <td>{row.buy.toFixed(2)}</td>
              <td>{row.sell.toFixed(2)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

type BuildingSort = 'name' | 'level' | 'staffing' | 'revenue' | 'cost' | 'profit' | 'shortages'

function BuildingsTable({ buildings }: { buildings: BuildingEconomics[] }) {
  const [sort, onSort] = useSort<BuildingSort>('profit', 'desc')
  const sorted = useMemo(
    () =>
      sortRows(buildings, sort, (building, key) => {
        if (key === 'name') return displayId(building.type_id)
        if (key === 'shortages') return building.short_inputs.length
        return building[key]
      }),
    [buildings, sort],
  )
  return (
    <div className="table-scroll">
      <table>
        <thead>
          <tr>
            <th><SortButton label="Building" sortKey="name" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="Levels" sortKey="level" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="Staffing" sortKey="staffing" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="Revenue" sortKey="revenue" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="Input cost" sortKey="cost" sort={sort} onSort={onSort} /></th>
            <th><SortButton label="Model profit" sortKey="profit" sort={sort} onSort={onSort} /></th>
            <th>Inputs</th>
            <th>Outputs</th>
            <th><SortButton label="Shortages" sortKey="shortages" sort={sort} onSort={onSort} /></th>
          </tr>
        </thead>
        <tbody>
          {sorted.map((building) => (
            <tr key={building.id}>
              <th title={building.type_id}>{displayId(building.type_id)}</th>
              <td>{building.level.toFixed(0)}</td>
              <td>{(building.staffing * 100).toFixed(0)}%</td>
              <td>{building.revenue.toFixed(2)}</td>
              <td>{building.cost.toFixed(2)}</td>
              <td>{building.profit.toFixed(2)}</td>
              <td>
                {building.inputs
                  .map((flow) => `${displayId(flow.good_id)} ${flow.quantity.toFixed(1)} (${flow.value.toFixed(2)})`)
                  .join(', ') || '—'}
              </td>
              <td>
                {building.outputs
                  .map((flow) => `${displayId(flow.good_id)} ${flow.quantity.toFixed(1)} (${flow.value.toFixed(2)})`)
                  .join(', ') || '—'}
              </td>
              <td>{building.short_inputs.map(displayId).join(', ') || '—'}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

export function PriceExplorer({
  result,
  scenario = false,
}: {
  result: PricesResult
  scenario?: boolean
}) {
  const [view, setView] = useState<View>(() => currentView())
  useEffect(() => {
    const update = () => setView(currentView())
    window.addEventListener('hashchange', update)
    return () => window.removeEventListener('hashchange', update)
  }, [])

  const states = result.states ?? []
  const stateGoods = result.state_goods ?? []
  const buildings = result.buildings ?? []

  if (view.kind === 'good') {
    const good = result.goods.find((row) => row.id === view.id)
    const rows = stateGoods
      .filter((row) => row.good_id === view.id)
      .map((row) => ({ ...row, state: states.find((state) => state.id === row.state_id) }))
    return (
      <section aria-labelledby="good-state-heading">
        <nav className="breadcrumbs" aria-label="Price detail">
          <a href="#/prices">Goods</a><span>›</span><span>{good ? goodName(good) : displayId(view.id)}</span>
        </nav>
        <div className="result-heading">
          <h2 id="good-state-heading">{good ? goodName(good) : displayId(view.id)} by state</h2>
          <span>{rows.length} states</span>
        </div>
        <p className="model-info">
          State buy/sell orders are attributed locally; price is the shared whole-save synthetic
          market price, not a MAPI local price.
        </p>
        {rows.length ? <StatesTable rows={rows} /> : <p>No state-attributed orders for this good.</p>}
      </section>
    )
  }

  if (view.kind === 'state') {
    const state = states.find((row) => row.id === view.id)
    const rows = buildings.filter((building) => building.state_id === view.id)
    const name = displayId(state?.region_id || `State ${view.id}`)
    return (
      <section aria-labelledby="state-buildings-heading">
        <nav className="breadcrumbs" aria-label="Price detail">
          <a href="#/prices">Goods</a><span>›</span><span>{name}</span>
        </nav>
        <div className="result-heading">
          <h2 id="state-buildings-heading">{name} buildings</h2>
          <span>{rows.length} buildings</span>
        </div>
        <p className="model-info">
          Revenue, costs, profit, and shortages are model estimates from production methods,
          staffing, and the shared synthetic price — not saved game cashflow fields.
        </p>
        {rows.length ? <BuildingsTable buildings={rows} /> : <p>No modeled buildings in this state.</p>}
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
      <GoodsTable goods={result.goods} />
    </section>
  )
}
