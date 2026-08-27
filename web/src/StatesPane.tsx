import { useEffect, useMemo, useState } from 'react'
import { GameIcon } from './GameIcon'
import {
  CountryFlag,
  ScopeFilter,
  SortButton,
  StatePage,
  displayId,
  sortRows,
  useSort,
  type FilterMode,
} from './PriceExplorer'
import type {
  Alert,
  BuildingEconomics,
  DefsIcons,
  PricesResult,
  StateInfo,
  StatePop,
  StateQualification,
  WorldDelta,
} from './types'
import { hashForState, parseStateId } from './workspaceNav'

type StateSort = 'name' | 'population' | 'market_access' | 'infrastructure' | 'sol' | 'shortages'

type StateListRow = {
  state: StateInfo
  name: string
  population: number
  marketAccess: number
  infrastructure: number | undefined
  infrastructureUsage: number | undefined
  sol: number | undefined
  jobShortages: StateQualification[]
  shortBuildings: BuildingEconomics[]
  shortageCount: number
}

function marketAccess(infrastructure?: number, usage?: number): number {
  if (infrastructure != null && usage != null && usage > 0) {
    return Math.min(1, Math.max(0, infrastructure / usage))
  }
  return 1
}

function popSize(pop: StatePop): number {
  return pop.demand_size ?? (pop.workforce ?? 0) + (pop.dependents ?? 0)
}

function solProxy(pops: StatePop[]): number | undefined {
  let weighted = 0
  let population = 0
  for (const pop of pops) {
    if (pop.wealth == null) continue
    const size = popSize(pop)
    if (size <= 0) continue
    weighted += pop.wealth * size
    population += size
  }
  return population > 0 ? weighted / population : undefined
}

function buildRows(result: PricesResult): StateListRow[] {
  const popsByState = new Map<number, StatePop[]>()
  for (const pop of result.state_pops ?? []) {
    const rows = popsByState.get(pop.state_id)
    if (rows) rows.push(pop)
    else popsByState.set(pop.state_id, [pop])
  }
  const qualsByState = new Map<number, StateQualification[]>()
  for (const row of result.state_qualifications ?? []) {
    const rows = qualsByState.get(row.state_id)
    if (rows) rows.push(row)
    else qualsByState.set(row.state_id, [row])
  }
  const buildingsByState = new Map<number, BuildingEconomics[]>()
  for (const building of result.buildings ?? []) {
    if (building.state_id == null) continue
    const rows = buildingsByState.get(building.state_id)
    if (rows) rows.push(building)
    else buildingsByState.set(building.state_id, [building])
  }

  return (result.states ?? []).map((state) => {
    const pops = popsByState.get(state.id) ?? []
    const qualifications = qualsByState.get(state.id) ?? []
    const buildings = buildingsByState.get(state.id) ?? []
    const jobShortages = qualifications.filter((row) => row.shortage > 0)
    const shortBuildings = buildings.filter((building) => building.short_inputs.length > 0)
    const workforce = pops.reduce((sum, pop) => sum + (pop.workforce ?? 0), 0)
    const dependents = pops.reduce((sum, pop) => sum + (pop.dependents ?? 0), 0)
    return {
      state,
      name: state.state_name || displayId(state.region_id || `State ${state.id}`),
      population: workforce + dependents,
      marketAccess: marketAccess(state.infrastructure, state.infrastructure_usage),
      infrastructure: state.infrastructure,
      infrastructureUsage: state.infrastructure_usage,
      sol: solProxy(pops),
      jobShortages,
      shortBuildings,
      shortageCount: jobShortages.length + shortBuildings.length,
    }
  })
}

function ShortageBadges({
  row,
  icons,
}: {
  row: StateListRow
  icons?: DefsIcons
}) {
  if (row.shortageCount === 0) return <>{'—'}</>
  return (
    <ul className="shortage-badges">
      <li className="shortage-badge">{row.shortageCount}</li>
      {row.jobShortages.map((job) => (
        <li key={`job-${job.name}`} className="shortage-badge" title={job.label || job.name}>
          <GameIcon kind="pop" id={job.name} icons={icons} />
          {job.label || displayId(job.name)}
        </li>
      ))}
      {row.shortBuildings.flatMap((building) =>
        building.short_inputs.map((good) => (
          <li key={`${building.id}-${good}`} className="shortage-badge" title={displayId(good)}>
            <GameIcon kind="good" id={good} icons={icons} />
            {displayId(good)}
          </li>
        )),
      )}
    </ul>
  )
}

export function StatesPane({
  result,
  icons = {},
  playerCountryId,
  playerMarketId,
  gated = false,
  alerts = [],
  onApply,
}: {
  result?: PricesResult
  icons?: DefsIcons
  playerCountryId?: number
  playerMarketId?: number
  gated?: boolean
  alerts?: Alert[]
  onApply?: (delta: WorldDelta) => void
}) {
  const [hash, setHash] = useState(() => window.location.hash)
  const [filterMode, setFilterMode] = useState<FilterMode>('our_market')
  const [sort, onSort] = useSort<StateSort>('name')
  useEffect(() => {
    const update = () => setHash(window.location.hash)
    window.addEventListener('hashchange', update)
    return () => window.removeEventListener('hashchange', update)
  }, [])

  const stateId = parseStateId(hash)
  const countries = result?.countries ?? []
  const missingPlayerMarket = filterMode === 'our_market' && playerMarketId == null
  const effectiveFilterMode: FilterMode = missingPlayerMarket ? 'all' : filterMode
  const inScope = (state: StateInfo) => {
    if (effectiveFilterMode === 'all') return true
    if (effectiveFilterMode === 'our_market') return state.market_id === playerMarketId
    return state.country_id === playerCountryId
  }
  const rows = useMemo(() => (result ? buildRows(result) : []), [result])
  const visible = useMemo(
    () => sortRows(rows.filter((row) => inScope(row.state)), sort, (row, key) => {
      if (key === 'name') return row.name
      if (key === 'population') return row.population
      if (key === 'market_access') return row.marketAccess
      if (key === 'infrastructure') return row.infrastructureUsage ?? -1
      if (key === 'sol') return row.sol ?? -1
      return row.shortageCount
    }),
    [rows, sort, effectiveFilterMode, playerCountryId, playerMarketId],
  )

  if (stateId != null && result) {
    return (
      <section className={gated ? 'workspace-page needs-defs' : 'workspace-page'}>
        <StatePage
          result={result}
          icons={icons}
          playerCountryId={playerCountryId}
          stateId={stateId}
          source="states"
          alerts={alerts}
          onApply={onApply}
        />
      </section>
    )
  }

  return (
    <section
      className={gated ? 'workspace-page needs-defs' : 'workspace-page'}
      aria-labelledby="states-heading"
    >
      <div className="result-heading">
        <h2 id="states-heading">States</h2>
        <span>{visible.length} states</span>
      </div>
      <ScopeFilter mode={effectiveFilterMode} onChange={setFilterMode} />
      {missingPlayerMarket && (
        <p className="model-info">Player market unavailable; showing all states.</p>
      )}
      {visible.length ? (
        <div className="table-scroll">
          <table className="states-table">
            <thead>
              <tr>
                <th><SortButton label="Name" sortKey="name" sort={sort} onSort={onSort} /></th>
                <th><SortButton label="Population" sortKey="population" sort={sort} onSort={onSort} /></th>
                <th><SortButton label="Market access" sortKey="market_access" sort={sort} onSort={onSort} /></th>
                <th><SortButton label="Infrastructure" sortKey="infrastructure" sort={sort} onSort={onSort} /></th>
                <th><SortButton label="SoL" sortKey="sol" sort={sort} onSort={onSort} /></th>
                <th><SortButton label="Shortages" sortKey="shortages" sort={sort} onSort={onSort} /></th>
              </tr>
            </thead>
            <tbody>
              {visible.map((row) => (
                <tr
                  key={row.state.id}
                  onClick={(event) => {
                    if ((event.target as HTMLElement).closest('a')) return
                    window.location.hash = hashForState(row.state.id)
                  }}
                >
                  <th>
                    <a className="state-link" href={hashForState(row.state.id)}>
                      <CountryFlag
                        countryId={row.state.country_id}
                        playerCountryId={playerCountryId}
                        countries={countries}
                      />
                      {row.name}
                    </a>
                  </th>
                  <td>{row.population.toLocaleString()}</td>
                  <td>{`${(row.marketAccess * 100).toFixed(0)}%`}</td>
                  <td>
                    {row.infrastructure != null
                      ? `${(row.infrastructureUsage ?? 0).toLocaleString()} / ${row.infrastructure.toLocaleString()}`
                      : '—'}
                  </td>
                  <td>{row.sol != null ? row.sol.toFixed(1) : '—'}</td>
                  <td><ShortageBadges row={row} icons={icons} /></td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <p>{result ? 'No states in this scope.' : 'Load a save to see states.'}</p>
      )}
    </section>
  )
}
