import { Fragment, useMemo, useState } from 'react'
import {
  LocalRecommendations,
  alertsForPops,
} from './AlertsPane'
import { GameIcon } from './GameIcon'
import {
  NeedBaskets,
  QualificationsTable,
  ScopeFilter,
  displayId,
  type FilterMode,
} from './PriceExplorer'
import type {
  Alert,
  BuildingEconomics,
  BuildingTypeInfo,
  DefsIcons,
  PricesResult,
  StateInfo,
  StateNeed,
  StatePop,
  WorldDelta,
} from './types'
import { hashForBuilding, hashForState } from './workspaceNav'

type ProfessionGroup = {
  id: string
  name: string
  pops: StatePop[]
  workforce: number
  dependents: number
}

function professionId(pop: StatePop): string {
  return pop.profession_name || 'unknown'
}

function professionName(pop: StatePop): string {
  return pop.profession_label || displayId(pop.profession_name || 'unknown')
}

function cultureName(pop: StatePop): string {
  return pop.culture_label || displayId(pop.culture_name || 'unknown')
}

function literacyLabel(pop: StatePop): string {
  if (pop.workforce && pop.workforce > 0 && pop.literate != null) {
    return `${((pop.literate / pop.workforce) * 100).toFixed(0)}%`
  }
  return '—'
}

function stateName(state?: StateInfo, stateId?: number): string {
  if (!state) return stateId != null ? `State ${stateId}` : 'State'
  return state.state_name || displayId(state.region_id || `State ${state.id}`)
}

function workplaceLabel(
  pop: StatePop,
  buildingsById: Map<number, BuildingEconomics>,
  typesById: Map<string, BuildingTypeInfo>,
): string | undefined {
  if (pop.workplace_id == null) return undefined
  const building = buildingsById.get(pop.workplace_id)
  if (!building) return `Building ${pop.workplace_id}`
  const type = typesById.get(building.type_id)
  return type?.name || displayId(building.type_id)
}

function groupPops(pops: StatePop[]): ProfessionGroup[] {
  const groups = new Map<string, ProfessionGroup>()
  for (const pop of pops) {
    const id = professionId(pop)
    const existing = groups.get(id)
    if (existing) {
      existing.pops.push(pop)
      existing.workforce += pop.workforce ?? 0
      existing.dependents += pop.dependents ?? 0
    } else {
      groups.set(id, {
        id,
        name: professionName(pop),
        pops: [pop],
        workforce: pop.workforce ?? 0,
        dependents: pop.dependents ?? 0,
      })
    }
  }
  return [...groups.values()].sort((left, right) => right.workforce - left.workforce || left.name.localeCompare(right.name))
}

function needsForState(stateId: number, stateNeeds: StateNeed[]) {
  return stateNeeds
    .filter((need) => need.state_id === stateId)
    .map((need) => ({
      need_name: need.need_name,
      need_label: need.need_label,
      package_value: need.package_value,
      goods: need.goods,
    }))
}

export function PopsPane({
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
  const [filterMode, setFilterMode] = useState<FilterMode>('our_market')
  const [openProfession, setOpenProfession] = useState<string | null>(null)
  const [openPop, setOpenPop] = useState<string | null>(null)

  const missingPlayerMarket = filterMode === 'our_market' && playerMarketId == null
  const effectiveFilterMode: FilterMode = missingPlayerMarket ? 'all' : filterMode
  const inScope = (state: StateInfo) => {
    if (effectiveFilterMode === 'all') return true
    if (effectiveFilterMode === 'our_market') return state.market_id === playerMarketId
    return state.country_id === playerCountryId
  }

  const states = result?.states ?? []
  const scopedStateIds = useMemo(
    () => new Set(states.filter(inScope).map((state) => state.id)),
    [states, effectiveFilterMode, playerCountryId, playerMarketId],
  )
  const statesById = useMemo(() => new Map(states.map((state) => [state.id, state])), [states])
  const buildingsById = useMemo(
    () => new Map((result?.buildings ?? []).map((building) => [building.id, building])),
    [result?.buildings],
  )
  const typesById = useMemo(
    () => new Map((result?.building_types ?? []).map((type) => [type.id, type])),
    [result?.building_types],
  )

  const pops = useMemo(
    () => (result?.state_pops ?? []).filter((pop) => scopedStateIds.has(pop.state_id)),
    [result?.state_pops, scopedStateIds],
  )
  const groups = useMemo(() => groupPops(pops), [pops])
  const shortages = useMemo(
    () => (result?.state_qualifications ?? []).filter(
      (row) => scopedStateIds.has(row.state_id) && row.shortage > 0,
    ),
    [result?.state_qualifications, scopedStateIds],
  )
  const scopedNeeds = useMemo(
    () => (result?.state_needs ?? []).filter((need) => scopedStateIds.has(need.state_id)),
    [result?.state_needs, scopedStateIds],
  )
  const popAlerts = useMemo(
    () =>
      alertsForPops(alerts).filter(
        (alert) => alert.state_id == null || scopedStateIds.has(alert.state_id),
      ),
    [alerts, scopedStateIds],
  )
  const needsByState = useMemo(() => {
    const ids = [...new Set(scopedNeeds.map((need) => need.state_id))]
    return ids.map((stateId) => ({
      stateId,
      name: stateName(statesById.get(stateId), stateId),
      needs: needsForState(stateId, scopedNeeds),
    }))
  }, [scopedNeeds, statesById])

  return (
    <section
      className={gated ? 'workspace-page needs-defs' : 'workspace-page'}
      aria-labelledby="pops-heading"
    >
      <div className="result-heading">
        <h2 id="pops-heading">Pops</h2>
        <span>{pops.length} pops</span>
      </div>
      <ScopeFilter mode={effectiveFilterMode} onChange={setFilterMode} />
      {missingPlayerMarket && (
        <p className="model-info">Player market unavailable; showing all pops.</p>
      )}
      <LocalRecommendations
        alerts={popAlerts}
        buildings={result?.buildings ?? []}
        icons={icons}
        onApply={onApply}
      />
      {shortages.length > 0 && (
        <section className="pop-shortages" aria-label="Qualification shortages">
          <h3>Qualification shortages</h3>
          <p className="model-info">
            Shortage is filled jobs minus employable (or qualified) stock. Monthly qualification gain is omitted unless the save stores it.
          </p>
          <QualificationsTable rows={shortages} />
        </section>
      )}
      {needsByState.length > 0 && (
        <section className="pop-state-needs" aria-label="Needs">
          <h3>Needs</h3>
          <p className="model-info">
            Needs are model baskets at solved prices (package ladder + substitution), not a save cashflow ledger.
          </p>
          {needsByState.map((row) => (
            <details key={row.stateId} className="pop-needs-expander">
              <summary>{row.name} needs</summary>
              <NeedBaskets needs={row.needs} goods={result?.goods ?? []} icons={icons} />
            </details>
          ))}
        </section>
      )}
      {groups.length ? (
        <ul className="pop-groups">
          {groups.map((group) => {
            const expanded = openProfession === group.id
            return (
              <li key={group.id} className="pop-group">
                <button
                  type="button"
                  className="pop-group-toggle"
                  aria-expanded={expanded}
                  onClick={() => {
                    setOpenProfession(expanded ? null : group.id)
                    setOpenPop(null)
                  }}
                >
                  {group.id !== 'unknown' ? (
                    <GameIcon kind="pop" id={group.id} icons={icons} />
                  ) : null}
                  {group.name}
                  <span className="pop-group-count">
                    {group.pops.length.toLocaleString()} pops · {group.workforce.toLocaleString()} workforce
                  </span>
                </button>
                {expanded && (
                  <div className="table-scroll">
                    <table className="pops-table">
                      <thead>
                        <tr>
                          <th>Profession</th>
                          <th>Culture</th>
                          <th>Workforce</th>
                          <th>Dependents</th>
                          <th>Literacy</th>
                          <th>Wealth</th>
                          <th>State</th>
                          <th>Workplace</th>
                        </tr>
                      </thead>
                      <tbody>
                        {group.pops.map((pop, index) => {
                          const popKey = `${group.id}-${pop.state_id}-${pop.culture_name ?? index}-${index}`
                          const popOpen = openPop === popKey
                          const workplace = workplaceLabel(pop, buildingsById, typesById)
                          const popNeeds = pop.needs ?? []
                          const fallbackNeeds = popNeeds.length ? popNeeds : needsForState(pop.state_id, scopedNeeds)
                          return (
                            <Fragment key={popKey}>
                              <tr>
                                <th>
                                  <button
                                    type="button"
                                    className="pop-expand"
                                    aria-expanded={popOpen}
                                    onClick={() => setOpenPop(popOpen ? null : popKey)}
                                  >
                                    {pop.profession_name ? (
                                      <GameIcon kind="pop" id={pop.profession_name} icons={icons} />
                                    ) : null}
                                    {professionName(pop)}
                                  </button>
                                </th>
                                <td>{cultureName(pop)}</td>
                                <td>{pop.workforce?.toLocaleString() ?? '—'}</td>
                                <td>{pop.dependents?.toLocaleString() ?? '—'}</td>
                                <td>{literacyLabel(pop)}</td>
                                <td>{pop.wealth ?? '—'}</td>
                                <td>
                                  <a className="state-link" href={hashForState(pop.state_id)}>
                                    {stateName(statesById.get(pop.state_id), pop.state_id)}
                                  </a>
                                </td>
                                <td>
                                  {pop.workplace_id != null ? (
                                    <a
                                      className="building-link"
                                      href={hashForBuilding(pop.workplace_id)}
                                    >
                                      {workplace}
                                    </a>
                                  ) : (
                                    '—'
                                  )}
                                </td>
                              </tr>
                              {popOpen && (
                                <tr className="pop-detail">
                                  <td colSpan={8}>
                                    <NeedBaskets
                                      needs={fallbackNeeds}
                                      goods={result?.goods ?? []}
                                      icons={icons}
                                    />
                                  </td>
                                </tr>
                              )}
                            </Fragment>
                          )
                        })}
                      </tbody>
                    </table>
                  </div>
                )}
              </li>
            )
          })}
        </ul>
      ) : (
        <p>{result ? 'No pops in this scope.' : 'Load a save to see pops.'}</p>
      )}
    </section>
  )
}
