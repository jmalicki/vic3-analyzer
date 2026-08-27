import { GameIcon } from './GameIcon'
import { deltaForMitigation } from './ConfirmApply'
import type {
  Alert,
  AlertKind,
  AlertsResult,
  BuildingEconomics,
  BuildingStaffing,
  DefsIcons,
  MitigationAction,
  StateInfo,
  WorldDelta,
} from './types'
import { hashForBuilding, hashForGood, hashForState } from './workspaceNav'

function formatStaffing(value: number): string {
  if (Number.isInteger(value) || Math.abs(value - Math.round(value)) < 1e-6) {
    return String(Math.round(value))
  }
  return value.toFixed(2)
}

function buildingStaffingSummary(row: BuildingStaffing): string {
  const percent =
    row.level > 0 ? ` (${Math.round((row.staffing / row.level) * 100)}%)` : ''
  const staffed = `${formatStaffing(row.staffing)} of ${formatStaffing(row.level)} levels staffed${percent}`
  const missing = [...row.professions]
    .filter((gap) => gap.missing_here > 0.05)
    .sort((left, right) => right.missing_here - left.missing_here)
  if (missing.length === 0) return staffed
  const parts = missing.map((gap) => {
    const name = gap.label || gap.name
    return `${formatStaffing(gap.missing_here)} more ${name}`
  })
  return `${staffed} — needs ${parts.join(', ')}`
}

function actionBuilding(action?: MitigationAction): string | undefined {
  if (!action) return undefined
  if (action.type === 'build' || action.type === 'feeder_job') return action.building
  return undefined
}

function actionGood(action?: MitigationAction): string | undefined {
  if (!action) return undefined
  if (action.type === 'trade_alloc' || action.type === 'sol_goods') return action.good_name
  return undefined
}

function actionPm(action?: MitigationAction): string | undefined {
  if (action?.type === 'pm') return action.production_method
  return undefined
}

function buildingIdsForAlert(alert: Alert): number[] {
  const ids = new Set<number>()
  if (alert.building_id != null) ids.add(alert.building_id)
  for (const row of alert.staffing ?? []) ids.add(row.building_id)
  for (const mitigation of alert.mitigations) {
    const action = mitigation.action
    if (action?.type === 'pm' || action?.type === 'subsidize') {
      ids.add(action.building_id)
    }
  }
  return [...ids]
}

function stateIdsForAlert(
  alert: Alert,
  buildingsById: Map<number, BuildingEconomics>,
): number[] {
  const ids = new Set<number>()
  if (alert.state_id != null) ids.add(alert.state_id)
  for (const buildingId of buildingIdsForAlert(alert)) {
    const stateId = buildingsById.get(buildingId)?.state_id
    if (stateId != null) ids.add(stateId)
  }
  for (const mitigation of alert.mitigations) {
    const action = mitigation.action
    if (
      action?.type === 'build' ||
      action?.type === 'feeder_job' ||
      action?.type === 'sol_goods' ||
      action?.type === 'trade_alloc'
    ) {
      if (action.state_id != null) ids.add(action.state_id)
    }
  }
  return [...ids]
}

function stateLabel(state?: StateInfo, stateId?: number): string {
  if (state?.label) return state.label
  const region = state?.region_name
  if (region) {
    return region
      .replace(/^STATE_/, '')
      .split('_')
      .filter(Boolean)
      .map((word) => word.charAt(0).toUpperCase() + word.slice(1).toLowerCase())
      .join(' ')
  }
  return stateId != null ? `State ${stateId}` : 'State'
}

export function alertIconId(kind: AlertKind): string {
  switch (kind) {
    case 'electricity_shortage':
      return 'electricity'
    case 'transportation_shortage':
      return 'transportation'
    case 'goods_shortage':
      return 'goods_shortage'
    case 'needs_unmet':
      return 'starving'
    case 'low_market_access':
      return 'world_market_access'
    case 'unfilled_education':
      return 'literacy'
    case 'unfilled_pops':
      return 'population'
    case 'underemployed':
      return 'population'
  }
}

function alertGroup(kind: AlertKind): { id: string; label: string } {
  switch (kind) {
    case 'electricity_shortage':
    case 'transportation_shortage':
    case 'goods_shortage':
      return { id: 'shortages', label: 'Shortages' }
    case 'needs_unmet':
      return { id: 'needs', label: 'Unmet needs' }
    case 'low_market_access':
      return { id: 'market_access', label: 'Market access' }
    case 'unfilled_education':
      return { id: 'qualifications', label: 'Qualifications' }
    case 'unfilled_pops':
    case 'underemployed':
      return { id: 'employment', label: 'Employment' }
  }
}

const GROUP_ORDER = ['shortages', 'needs', 'market_access', 'qualifications', 'employment']

export function hrefForAlert(alert: Alert): string {
  switch (alert.kind) {
    case 'unfilled_pops':
    case 'underemployed':
      if (alert.building_id != null) return hashForBuilding(alert.building_id)
      if (alert.state_id != null) return hashForState(alert.state_id)
      return '#/buildings'
    case 'electricity_shortage':
    case 'transportation_shortage':
    case 'goods_shortage':
      if (alert.good_name) return hashForGood(alert.good_name)
      if (alert.building_id != null) return hashForBuilding(alert.building_id)
      if (alert.state_id != null) return hashForState(alert.state_id)
      return '#/prices'
    case 'needs_unmet':
    case 'low_market_access':
      if (alert.state_id != null) return hashForState(alert.state_id)
      return '#/states'
    case 'unfilled_education':
      if (alert.state_id != null) return hashForState(alert.state_id)
      return '#/pops'
  }
}

export function alertsForBuilding(alerts: Alert[], building: BuildingEconomics): Alert[] {
  const goods = new Set([
    ...(building.short_inputs ?? []),
    ...(building.inputs ?? []).map((flow) => flow.name),
    ...(building.outputs ?? []).map((flow) => flow.name),
  ])
  return alerts.filter((alert) => {
    if (alert.building_id === building.id) return true
    if (alert.staffing?.some((row) => row.building_id === building.id)) return true
    if (
      alert.mitigations.some(
        (item) => item.action?.type === 'pm' && item.action.building_id === building.id,
      )
    ) {
      return true
    }
    if (
      alert.good_name &&
      goods.has(alert.good_name) &&
      (alert.state_id == null || alert.state_id === building.state_id)
    ) {
      return (
        alert.kind === 'electricity_shortage' ||
        alert.kind === 'transportation_shortage' ||
        alert.kind === 'goods_shortage'
      )
    }
    return false
  })
}

export function alertsForState(
  alerts: Alert[],
  stateId: number,
  buildings: BuildingEconomics[] = [],
): Alert[] {
  const ids = new Set(
    buildings.filter((building) => building.state_id === stateId).map((building) => building.id),
  )
  return alerts.filter(
    (alert) =>
      alert.state_id === stateId ||
      (alert.building_id != null && ids.has(alert.building_id)) ||
      alert.staffing?.some((row) => ids.has(row.building_id)),
  )
}

export function alertsForGood(alerts: Alert[], goodId: string): Alert[] {
  return alerts.filter((alert) => alert.good_name === goodId)
}

export function alertsForPops(alerts: Alert[]): Alert[] {
  return alerts.filter(
    (alert) =>
      alert.kind === 'unfilled_education' ||
      alert.kind === 'unfilled_pops' ||
      alert.kind === 'underemployed' ||
      alert.kind === 'needs_unmet',
  )
}

export function AlertsPane({
  result,
  icons,
  states = [],
  buildings = [],
  playerCountryId,
}: {
  result: AlertsResult
  icons?: DefsIcons
  states?: StateInfo[]
  buildings?: BuildingEconomics[]
  /** When set, hide alerts tied to foreign states (same rule as SQL `alerts()`). */
  playerCountryId?: number
}) {
  const statesById = new Map(states.map((state) => [state.id, state]))
  const scopedAlerts =
    playerCountryId == null
      ? result.alerts
      : result.alerts.filter((alert) => {
          if (alert.state_id == null) return true
          const state = statesById.get(alert.state_id)
          return state?.country_id === playerCountryId
        })
  if (scopedAlerts.length === 0) {
    return <p>No shortages detected in the current solve.</p>
  }
  const buildingsById = new Map(buildings.map((building) => [building.id, building]))
  const groups = GROUP_ORDER.flatMap((id) => {
    const items = scopedAlerts.filter((alert) => alertGroup(alert.kind).id === id)
    if (!items.length) return []
    return [{ id, label: alertGroup(items[0].kind).label, items }]
  })
  return (
    <ul className="alert-groups">
      {groups.map((group) => (
        <li key={group.id}>
          <details className="alert-group" open>
            <summary>
              <span className="alert-heading">
                <GameIcon kind="alert" id={alertIconId(group.items[0].kind)} icons={icons} />
                <strong>{group.label}</strong>
                <span className="alert-severity">
                  {group.items.length} alert{group.items.length === 1 ? '' : 's'}
                </span>
              </span>
            </summary>
            <ul className="alerts-list">
              {group.items.map((alert) => (
                <li key={alert.id}>
                  <AlertIndexRow
                    alert={alert}
                    icons={icons}
                    statesById={statesById}
                    buildingsById={buildingsById}
                  />
                </li>
              ))}
            </ul>
          </details>
        </li>
      ))}
    </ul>
  )
}

function AlertIndexRow({
  alert,
  icons,
  statesById,
  buildingsById,
}: {
  alert: Alert
  icons?: DefsIcons
  statesById: Map<number, StateInfo>
  buildingsById: Map<number, BuildingEconomics>
}) {
  const href = hrefForAlert(alert)
  const places = stateIdsForAlert(alert, buildingsById).filter(
    (stateId) => hashForState(stateId) !== href,
  )
  return (
    <div className="alert-index-row">
      <a className="alert-index-link" href={href}>
        <GameIcon kind="alert" id={alertIconId(alert.kind)} icons={icons} />
        {alert.good_name && <GameIcon kind="good" id={alert.good_name} icons={icons} />}
        <strong>{alert.title}</strong>
        <span className="alert-severity">severity {alert.severity}</span>
      </a>
      {places.map((stateId) => (
        <a key={stateId} className="alert-index-place" href={hashForState(stateId)}>
          {stateLabel(statesById.get(stateId), stateId)}
        </a>
      ))}
    </div>
  )
}

export function LocalRecommendations({
  alerts,
  buildings = [],
  icons,
  onApply,
  heading = 'Recommendations',
}: {
  alerts: Alert[]
  buildings?: BuildingEconomics[]
  icons?: DefsIcons
  onApply?: (delta: WorldDelta) => void
  heading?: string
}) {
  if (!alerts.length) return null
  return (
    <section className="local-alerts" aria-label={heading}>
      <h3>{heading}</h3>
      <ul className="alerts-list">
        {alerts.map((alert) => (
          <li key={alert.id}>
            <LocalAlertCard
              alert={alert}
              icons={icons}
              buildings={buildings}
              onApply={onApply}
            />
          </li>
        ))}
      </ul>
    </section>
  )
}

function LocalAlertCard({
  alert,
  icons,
  buildings,
  onApply,
}: {
  alert: Alert
  icons?: DefsIcons
  buildings: BuildingEconomics[]
  onApply?: (delta: WorldDelta) => void
}) {
  const mitigations = [...alert.mitigations].sort((left, right) => left.rank - right.rank)
  return (
    <details className="alert-expander">
      <summary>
        <span className="alert-heading">
          <GameIcon kind="alert" id={alertIconId(alert.kind)} icons={icons} />
          {alert.good_name && <GameIcon kind="good" id={alert.good_name} icons={icons} />}
          <strong>{alert.title}</strong>
          <span className="alert-severity">severity {alert.severity}</span>
        </span>
        <span className="alert-summary">{alert.summary}</span>
      </summary>
      {alert.evidence.length > 0 && (
        <dl className="alert-evidence">
          {alert.evidence.map((row) => (
            <div key={`${row.label}:${row.value}`}>
              <dt>{row.label}</dt>
              <dd>{row.value}</dd>
            </div>
          ))}
        </dl>
      )}
      {(alert.staffing?.length ?? 0) > 0 && (
        <ul className="alert-staffing">
          {alert.staffing!.map((row) => (
            <li key={row.building_id}>
              <details open>
                <summary>
                  <GameIcon kind="building" id={row.building_type_name} icons={icons} />
                  <strong>{row.building_type_label}</strong>
                  <span>{buildingStaffingSummary(row)}</span>
                </summary>
                <p className="alert-staffing-link">
                  <a href={hashForBuilding(row.building_id)}>Open building</a>
                </p>
                {row.professions.length === 0 ? (
                  <p>
                    {row.staffing <= 0.05
                      ? 'This building has no workers yet, so the save does not list which professions it needs.'
                      : 'No employee counts on this building, so the missing profession mix is unknown.'}
                  </p>
                ) : (
                  <ul className="alert-profession-gaps">
                    {row.professions.map((gap) => {
                      const name = gap.label || gap.name
                      const blocking = gap.state_shortage > 0 && gap.missing_here > 0
                      return (
                        <li key={gap.name} className={blocking ? 'blocking' : undefined}>
                          <GameIcon kind="pop" id={gap.name} icons={icons} />
                          <span>
                            <strong>{name}</strong>
                            {blocking ? ' — this is blocking' : ''}
                            : {formatStaffing(gap.employed_here)} working here,{' '}
                            {formatStaffing(gap.jobs_here)} needed to staff this building
                            {gap.missing_here > 0.05
                              ? ` (${formatStaffing(gap.missing_here)} more)`
                              : ' (enough here)'}
                            . State: {formatStaffing(gap.state_jobs)} jobs,{' '}
                            {formatStaffing(gap.state_stock)} people who can take them
                            {gap.state_shortage > 0.05
                              ? ` — short ${formatStaffing(gap.state_shortage)}`
                              : ' — enough in the state'}
                            .
                          </span>
                        </li>
                      )
                    })}
                  </ul>
                )}
              </details>
            </li>
          ))}
        </ul>
      )}
      <ol className="alert-mitigations">
        {mitigations.map((mitigation) => {
          const delta = deltaForMitigation(mitigation.action, buildings)
          const canApply = Boolean(delta && onApply)
          return (
            <li key={mitigation.id}>
              <div className="alert-mitigation-heading">
                {actionBuilding(mitigation.action) && (
                  <GameIcon kind="building" id={actionBuilding(mitigation.action)!} icons={icons} />
                )}
                {actionGood(mitigation.action) && (
                  <GameIcon kind="good" id={actionGood(mitigation.action)!} icons={icons} />
                )}
                {actionPm(mitigation.action) && (
                  <GameIcon kind="pm" id={actionPm(mitigation.action)!} icons={icons} />
                )}
                <strong>
                  {mitigation.rank}. {mitigation.title}
                </strong>
                <button
                  type="button"
                  className="alert-apply"
                  disabled={!canApply}
                  title={canApply ? 'Apply this mitigation' : 'Cannot apply this mitigation yet'}
                  onClick={() => {
                    if (delta) onApply?.(delta)
                  }}
                >
                  Apply
                </button>
              </div>
              <p>{mitigation.detail}</p>
              {mitigation.effect && (
                <p className="alert-mitigation-effect">Estimated effect: {mitigation.effect}</p>
              )}
            </li>
          )
        })}
      </ol>
    </details>
  )
}
