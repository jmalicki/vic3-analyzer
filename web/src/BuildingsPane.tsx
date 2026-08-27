import { useEffect, useMemo, useState } from 'react'
import { GameIcon } from './GameIcon'
import {
  BuildingPage,
  displayId,
  ScopeFilter,
  SortButton,
  sortRows,
  useSort,
  type FilterMode,
} from './PriceExplorer'
import type {
  Alert,
  BuildingEconomics,
  BuildingTypeInfo,
  DefsIcons,
  GoodFlow,
  OptimizeAxis,
  OptimizeChange,
  OptimizeResult,
  PricesResult,
  ProductionMethodDef,
  StateInfo,
  WorldDelta,
} from './types'
import type { WasmApi } from './wasm'
import { hashForBuilding, parseBuildingId } from './workspaceNav'

type BuildingSort = 'name' | 'productivity' | 'profit' | 'employment' | 'shortage'

type TypeRow = {
  typeId: string
  name: string
  buildings: BuildingEconomics[]
  levels: number
  staffing: number
  profit: number
  productivity: number
  employment: number
  shortage: number
  productionMethodIds: string[]
}

function employmentRatio(staffing: number, level: number): number {
  if (level <= 0) return 0
  return Math.max(0, Math.min(1, staffing / level))
}

function uniqueIds(ids: string[]): string[] {
  return [...new Set(ids)]
}

const HIDDEN_FROM_UI = /should never show up in the UI/i

function isHiddenFromUi(name?: string): boolean {
  return Boolean(name && HIDDEN_FROM_UI.test(name))
}

function buildRows(result: PricesResult, buildings: BuildingEconomics[]): TypeRow[] {
  const types = new Map((result.building_types ?? []).map((type) => [type.id, type]))
  const grouped = new Map<string, BuildingEconomics[]>()
  for (const building of buildings) {
    const rows = grouped.get(building.type_id)
    if (rows) rows.push(building)
    else grouped.set(building.type_id, [building])
  }
  const rows: TypeRow[] = []
  for (const [typeId, groupedBuildings] of grouped) {
    const type: BuildingTypeInfo | undefined = types.get(typeId)
    const name = type?.name || displayId(typeId)
    if (isHiddenFromUi(type?.name) || isHiddenFromUi(name)) continue
    const levels = groupedBuildings.reduce((sum, building) => sum + building.level, 0)
    const staffing = groupedBuildings.reduce((sum, building) => sum + building.staffing, 0)
    const profit = groupedBuildings.reduce((sum, building) => sum + building.profit, 0)
    rows.push({
      typeId,
      name,
      buildings: groupedBuildings,
      levels,
      staffing,
      profit,
      productivity: levels > 0 ? profit / levels : 0,
      employment: employmentRatio(staffing, levels),
      shortage: groupedBuildings.reduce((sum, building) => sum + building.short_inputs.length, 0),
      productionMethodIds: uniqueIds(
        groupedBuildings.flatMap((building) => building.production_method_ids ?? []),
      ),
    })
  }
  return rows
}

function stateName(result: PricesResult, stateId?: number): string {
  const state = result.states?.find((row) => row.id === stateId)
  return state?.label || displayId(state?.region_name || (stateId != null ? `State ${stateId}` : 'State'))
}

function sameIds(left: string[], right: string[]): boolean {
  if (left.length !== right.length) return false
  const other = new Set(right)
  return left.every((id) => other.has(id))
}

function formatDelta(value: number): string {
  const formatted = value.toFixed(2)
  if (value > 0) return `+${formatted}`
  return formatted
}

function groupOptimizeChanges(changes: OptimizeChange[]): { typeId: string; changes: OptimizeChange[] }[] {
  const grouped = new Map<string, OptimizeChange[]>()
  for (const change of changes) {
    const rows = grouped.get(change.building_type)
    if (rows) rows.push(change)
    else grouped.set(change.building_type, [change])
  }
  return [...grouped.entries()].map(([typeId, groupedChanges]) => ({
    typeId,
    changes: groupedChanges,
  }))
}

function previewFlows(
  selected: string[],
  building: BuildingEconomics,
  methods: ProductionMethodDef[],
): { inputs: GoodFlow[]; outputs: GoodFlow[] } {
  const byId = new Map(methods.map((method) => [method.id, method]))
  const hasRecipes = selected.some((id) => byId.has(id))
  if (!hasRecipes) {
    const current = building.production_method_ids ?? []
    if (sameIds(selected, current)) {
      return { inputs: building.inputs, outputs: building.outputs }
    }
    return { inputs: [], outputs: [] }
  }
  const inputs = new Map<string, number>()
  const outputs = new Map<string, number>()
  for (const id of selected) {
    const method = byId.get(id)
    if (!method) continue
    for (const flow of method.inputs) {
      inputs.set(flow.good, (inputs.get(flow.good) ?? 0) + flow.qty)
    }
    for (const flow of method.outputs) {
      outputs.set(flow.good, (outputs.get(flow.good) ?? 0) + flow.qty)
    }
  }
  const toFlows = (map: Map<string, number>): GoodFlow[] =>
    [...map.entries()].map(([name, quantity]) => ({ name, quantity, value: 0 }))
  return { inputs: toFlows(inputs), outputs: toFlows(outputs) }
}

function FlowList({
  flows,
  icons,
}: {
  flows: GoodFlow[]
  icons?: DefsIcons
}) {
  if (!flows.length) return <>{'—'}</>
  return (
    <ul className="good-chips">
      {flows.map((flow) => (
        <li key={flow.name}>
          <GameIcon kind="good" id={flow.name} icons={icons} />
          {displayId(flow.name)} {flow.quantity.toFixed(1)}
        </li>
      ))}
    </ul>
  )
}

function PmList({
  ids,
  icons,
}: {
  ids: string[]
  icons?: DefsIcons
}) {
  if (!ids.length) return <>{'—'}</>
  return (
    <ul className="pm-list">
      {ids.map((id) => (
        <li key={id}>
          <GameIcon kind="pm" id={id} icons={icons} />
          {displayId(id)}
        </li>
      ))}
    </ul>
  )
}

export function BuildingsPane({
  result,
  icons = {},
  playerCountryId,
  playerMarketId,
  gated = false,
  api,
  onWhatIf,
  onApply,
  productionMethods,
  alerts = [],
  /** When true, App owns the page chrome (heading / tabs); skip nested section + h2. */
  embedded = false,
}: {
  result?: PricesResult
  icons?: DefsIcons
  playerCountryId?: number
  playerMarketId?: number
  gated?: boolean
  api?: WasmApi
  onWhatIf?: (building: string, extraLevels: number) => void
  onApply?: (delta: WorldDelta) => void
  productionMethods?: ProductionMethodDef[]
  alerts?: Alert[]
  embedded?: boolean
}) {
  const [hash, setHash] = useState(() => window.location.hash)
  const [filterMode, setFilterMode] = useState<FilterMode>('domestic')
  const [sort, onSort] = useSort<BuildingSort>('name')
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set())
  const [extraLevels, setExtraLevels] = useState<Record<string, number>>({})
  const [selectedPms, setSelectedPms] = useState<Record<number, string[]>>({})
  const [loadedMethods, setLoadedMethods] = useState<ProductionMethodDef[]>([])
  const [axis, setAxis] = useState<OptimizeAxis>('productivity')
  const [optimizeResult, setOptimizeResult] = useState<OptimizeResult>()
  const [optimizeError, setOptimizeError] = useState<string>()
  const [optimizing, setOptimizing] = useState(false)
  const [expandedOptimize, setExpandedOptimize] = useState<Set<string>>(() => new Set())
  useEffect(() => {
    const update = () => setHash(window.location.hash)
    window.addEventListener('hashchange', update)
    return () => window.removeEventListener('hashchange', update)
  }, [])
  useEffect(() => {
    if (productionMethods || !api?.loaded_production_methods) return
    let cancelled = false
    void Promise.resolve(api.loaded_production_methods())
      .then((json) => {
        if (!cancelled) setLoadedMethods(JSON.parse(json) as ProductionMethodDef[])
      })
      .catch(() => {
        if (!cancelled) setLoadedMethods([])
      })
    return () => {
      cancelled = true
    }
  }, [api, productionMethods])

  const buildingId = parseBuildingId(hash)
  const methods = productionMethods ?? loadedMethods
  const missingPlayerMarket = filterMode === 'our_market' && playerMarketId == null
  const missingPlayerCountry = filterMode === 'domestic' && playerCountryId == null
  const effectiveFilterMode: FilterMode =
    missingPlayerMarket || missingPlayerCountry ? 'all' : filterMode
  const rows = useMemo(() => {
    if (!result) return []
    const states = new Map((result.states ?? []).map((state) => [state.id, state]))
    const inScope = (state?: StateInfo) => {
      if (effectiveFilterMode === 'all') return true
      if (!state) return false
      if (effectiveFilterMode === 'our_market') return state.market_id === playerMarketId
      return state.country_id === playerCountryId
    }
    const buildings = (result.buildings ?? []).filter((building) =>
      inScope(building.state_id == null ? undefined : states.get(building.state_id)),
    )
    return buildRows(result, buildings)
  }, [result, effectiveFilterMode, playerCountryId, playerMarketId])
  const visible = useMemo(
    () =>
      sortRows(rows, sort, (row, key) => {
        if (key === 'name') return row.name
        if (key === 'productivity') return row.productivity
        if (key === 'profit') return row.profit
        if (key === 'employment') return row.employment
        return row.shortage
      }),
    [rows, sort],
  )

  if (buildingId != null && result) {
    const detail = (
      <BuildingPage
        result={result}
        icons={icons}
        buildingId={buildingId}
        source="buildings"
        alerts={alerts}
        onApply={onApply}
      />
    )
    if (embedded) return detail
    return (
      <section className={gated ? 'workspace-page needs-defs' : 'workspace-page'}>{detail}</section>
    )
  }

  const toggleType = (typeId: string) => {
    setExpanded((current) => {
      const next = new Set(current)
      if (next.has(typeId)) next.delete(typeId)
      else next.add(typeId)
      return next
    })
  }

  const selectedFor = (building: BuildingEconomics): string[] =>
    selectedPms[building.id] ?? building.production_method_ids ?? []

  const canOptimize = Boolean(api?.loaded_optimize_pms)
  const runOptimize = () => {
    if (!api?.loaded_optimize_pms) return
    setOptimizing(true)
    setOptimizeError(undefined)
    void Promise.resolve(api.loaded_optimize_pms(JSON.stringify({ axis })))
      .then((json) => {
        setOptimizeResult(JSON.parse(json) as OptimizeResult)
      })
      .catch((reason: unknown) => {
        setOptimizeResult(undefined)
        setOptimizeError(reason instanceof Error ? reason.message : String(reason))
      })
      .finally(() => {
        setOptimizing(false)
      })
  }

  const toggleOptimizeType = (typeId: string) => {
    setExpandedOptimize((current) => {
      const next = new Set(current)
      if (next.has(typeId)) next.delete(typeId)
      else next.add(typeId)
      return next
    })
  }

  const typeName = (typeId: string): string =>
    result?.building_types?.find((type) => type.id === typeId)?.name || displayId(typeId)

  const content = (
    <>
      <div className="result-heading">
        {!embedded && <h2 id="buildings-heading">Buildings</h2>}
        <div className="building-what-if">
          <label>
            Optimize for
            <select
              aria-label="Optimization axis"
              value={axis}
              onChange={(event) => setAxis(event.target.value as OptimizeAxis)}
            >
              <option value="productivity">Productivity</option>
              <option value="income">Income</option>
              <option value="sol">SoL</option>
            </select>
          </label>
          <button type="button" disabled={!canOptimize || optimizing} onClick={runOptimize}>
            Optimize production methods
          </button>
        </div>
      </div>
      <ScopeFilter mode={effectiveFilterMode} onChange={setFilterMode} />
      {missingPlayerMarket && (
        <p className="model-info">Player market unavailable; showing all buildings.</p>
      )}
      {missingPlayerCountry && (
        <p className="model-info">Player country unavailable; showing all buildings.</p>
      )}
      {optimizeError && <p role="alert">{optimizeError}</p>}
      {optimizeResult && (
        <OptimizeDiff
          result={optimizeResult}
          typeName={typeName}
          expanded={expandedOptimize}
          onToggle={toggleOptimizeType}
          onApply={onApply}
        />
      )}
      {visible.length ? (
        <div className="table-scroll buildings-table-scroll">
          <table className="buildings-table">
            <thead>
              <tr>
                <th><SortButton label="Name" sortKey="name" sort={sort} onSort={onSort} /></th>
                <th><SortButton label="Productivity" sortKey="productivity" sort={sort} onSort={onSort} /></th>
                <th><SortButton label="Profit" sortKey="profit" sort={sort} onSort={onSort} /></th>
                <th><SortButton label="Employment" sortKey="employment" sort={sort} onSort={onSort} /></th>
                <th><SortButton label="Shortage" sortKey="shortage" sort={sort} onSort={onSort} /></th>
                <th>What-if</th>
              </tr>
            </thead>
            <tbody>
              {visible.map((row) => {
                const open = expanded.has(row.typeId)
                const extra = extraLevels[row.typeId] ?? 1
                return (
                  <TypeBlock
                    key={row.typeId}
                    row={row}
                    open={open}
                    extra={extra}
                    result={result}
                    icons={icons}
                    pickerIds={row.productionMethodIds}
                    methods={methods}
                    selectedFor={selectedFor}
                    onToggle={() => toggleType(row.typeId)}
                    onExtra={(value) =>
                      setExtraLevels((current) => ({ ...current, [row.typeId]: value }))
                    }
                    onWhatIf={onWhatIf}
                    onApply={onApply}
                    onSelectPm={(building, id, checked) => {
                      setSelectedPms((current) => {
                        const previous = current[building.id] ?? building.production_method_ids ?? []
                        const next = checked
                          ? uniqueIds([...previous, id])
                          : previous.filter((item) => item !== id)
                        return { ...current, [building.id]: next }
                      })
                    }}
                  />
                )
              })}
            </tbody>
          </table>
        </div>
      ) : (
        <p>{result ? 'No buildings in this save.' : 'Load a save to see buildings.'}</p>
      )}
    </>
  )

  if (embedded) return content
  return (
    <section
      className={gated ? 'workspace-page needs-defs' : 'workspace-page'}
      aria-labelledby="buildings-heading"
    >
      {content}
    </section>
  )
}

function TypeBlock({
  row,
  open,
  extra,
  result,
  icons,
  pickerIds,
  methods,
  selectedFor,
  onToggle,
  onExtra,
  onWhatIf,
  onApply,
  onSelectPm,
}: {
  row: TypeRow
  open: boolean
  extra: number
  result?: PricesResult
  icons?: DefsIcons
  pickerIds: string[]
  methods: ProductionMethodDef[]
  selectedFor: (building: BuildingEconomics) => string[]
  onToggle: () => void
  onExtra: (value: number) => void
  onWhatIf?: (building: string, extraLevels: number) => void
  onApply?: (delta: WorldDelta) => void
  onSelectPm: (building: BuildingEconomics, id: string, checked: boolean) => void
}) {
  return (
    <>
      <tr>
        <th>
          <div className="building-type-name">
            <button
              type="button"
              className="building-expand"
              aria-expanded={open}
              aria-label={`${open ? 'Collapse' : 'Expand'} ${row.name}`}
              onClick={onToggle}
            >
              {open ? '▼' : '▶'}
            </button>
            <GameIcon kind="building" id={row.typeId} icons={icons} />
            {row.name}
          </div>
        </th>
        <td>{row.productivity.toFixed(2)}</td>
        <td>{row.profit.toFixed(2)}</td>
        <td>{`${(row.employment * 100).toFixed(0)}%`}</td>
        <td>{row.shortage || '—'}</td>
        <td>
          <div className="building-what-if">
            <label>
              Extra levels
              <input
                aria-label={`Extra levels for ${row.name}`}
                type="number"
                min="1"
                step="1"
                value={extra}
                onChange={(event) => onExtra(Number(event.target.value))}
              />
            </label>
            <button
              type="button"
              disabled={!onWhatIf}
              onClick={() => onWhatIf?.(row.typeId, extra)}
            >
              Run what-if
            </button>
            <button
              type="button"
              aria-label={`Apply extra levels for ${row.name}`}
              disabled={!onApply || extra < 1}
              onClick={() =>
                onApply?.({ extra_levels: [{ building: row.typeId, extra_levels: extra }] })
              }
            >
              Apply
            </button>
          </div>
        </td>
      </tr>
      {open && (
        <tr className="building-type-detail">
          <td colSpan={6}>
            <PmList ids={row.productionMethodIds} icons={icons} />
            <div className="table-scroll">
              <table className="building-instances">
                <thead>
                  <tr>
                    <th>State</th>
                    <th>Levels</th>
                    <th>Employment</th>
                    <th>Profit</th>
                    <th>Production methods</th>
                  </tr>
                </thead>
                <tbody>
                  {row.buildings.map((building) => {
                    const selected = selectedFor(building)
                    const preview = previewFlows(selected, building, methods)
                    const instanceName = result
                      ? stateName(result, building.state_id)
                      : `Building ${building.id}`
                    return (
                      <tr key={building.id}>
                        <th>
                          <a className="building-link" href={hashForBuilding(building.id)}>
                            {instanceName}
                          </a>
                          <span className="building-levels">{building.level.toLocaleString()} levels</span>
                        </th>
                        <td>{building.level.toLocaleString()}</td>
                        <td>{`${(employmentRatio(building.staffing, building.level) * 100).toFixed(0)}%`}</td>
                        <td>{building.profit.toFixed(2)}</td>
                        <td>
                          <PmList ids={building.production_method_ids ?? []} icons={icons} />
                          <fieldset className="pm-picker">
                            <legend>Production method preview</legend>
                            {pickerIds.map((id) => (
                              <label key={id}>
                                <input
                                  type="checkbox"
                                  aria-label={`${displayId(id)} for ${instanceName}`}
                                  checked={selected.includes(id)}
                                  onChange={(event) => onSelectPm(building, id, event.target.checked)}
                                />
                                <GameIcon kind="pm" id={id} icons={icons} />
                                {displayId(id)}
                              </label>
                            ))}
                            <p className="pm-preview-note">
                              Preview uses selected methods; Apply writes them to a new save step.
                            </p>
                            <button
                              type="button"
                              aria-label={`Apply production methods for ${instanceName}`}
                              disabled={
                                !onApply ||
                                sameIds(selected, building.production_method_ids ?? []) ||
                                selected.length === 0
                              }
                              onClick={() =>
                                onApply?.({
                                  production_methods: [{ building_id: building.id, methods: selected }],
                                })
                              }
                            >
                              Apply
                            </button>
                            <dl className="pm-preview">
                              <div>
                                <dt>Inputs</dt>
                                <dd><FlowList flows={preview.inputs} icons={icons} /></dd>
                              </div>
                              <div>
                                <dt>Outputs</dt>
                                <dd><FlowList flows={preview.outputs} icons={icons} /></dd>
                              </div>
                            </dl>
                          </fieldset>
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            </div>
          </td>
        </tr>
      )}
    </>
  )
}

function OptimizeDiff({
  result,
  typeName,
  expanded,
  onToggle,
  onApply,
}: {
  result: OptimizeResult
  typeName: (typeId: string) => string
  expanded: Set<string>
  onToggle: (typeId: string) => void
  onApply?: (delta: WorldDelta) => void
}) {
  const groups = groupOptimizeChanges(result.changes)
  const methodsFor = (ids: number[]) =>
    (result.world_delta.production_methods ?? []).filter((item) => ids.includes(item.building_id))
  return (
    <div className="alert-expander">
      <p>
        Estimated Δ: income {formatDelta(result.delta.income)}, productivity{' '}
        {formatDelta(result.delta.productivity)}, SoL {formatDelta(result.delta.sol)}
      </p>
      {groups.length ? (
        <>
          <button
            type="button"
            className="alert-apply"
            disabled={!onApply}
            onClick={() => onApply?.(result.world_delta)}
          >
            Apply all
          </button>
          <ul className="alerts-list">
            {groups.map((group) => {
              const open = expanded.has(group.typeId)
              const groupIds = group.changes.map((change) => change.building_id)
              return (
                <li key={group.typeId}>
                  <div className="alert-mitigation-heading">
                    <button
                      type="button"
                      className="building-expand"
                      aria-expanded={open}
                      aria-label={`${open ? 'Collapse' : 'Expand'} ${typeName(group.typeId)} changes`}
                      onClick={() => onToggle(group.typeId)}
                    >
                      {open ? '▼' : '▶'}
                    </button>
                    <strong>{typeName(group.typeId)}</strong>
                    <span className="alert-severity">
                      {group.changes.length} building{group.changes.length === 1 ? '' : 's'}
                    </span>
                    <button
                      type="button"
                      className="alert-apply"
                      disabled={!onApply}
                      onClick={() => onApply?.({ production_methods: methodsFor(groupIds) })}
                    >
                      Apply
                    </button>
                  </div>
                  {open && (
                    <ul className="archive-list">
                      {group.changes.map((change) => (
                        <li key={change.building_id}>
                          <span>Building {change.building_id}</span>
                          <span>
                            {change.from.map(displayId).join(', ') || '—'} →{' '}
                            {change.to.map(displayId).join(', ') || '—'}
                          </span>
                          <button
                            type="button"
                            disabled={!onApply}
                            onClick={() =>
                              onApply?.({
                                production_methods: methodsFor([change.building_id]),
                              })
                            }
                          >
                            Apply
                          </button>
                        </li>
                      ))}
                    </ul>
                  )}
                </li>
              )
            })}
          </ul>
        </>
      ) : (
        <p>No improving production-method changes found.</p>
      )}
    </div>
  )
}
