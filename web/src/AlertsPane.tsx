import { GameIcon } from './GameIcon'
import { deltaForMitigation } from './ConfirmApply'
import type {
  Alert,
  AlertKind,
  AlertsResult,
  BuildingEconomics,
  DefsIcons,
  MitigationAction,
  WorldDelta,
} from './types'

function actionBuilding(action?: MitigationAction): string | undefined {
  if (!action) return undefined
  if (action.type === 'build' || action.type === 'feeder_job') return action.building
  return undefined
}

function actionGood(action?: MitigationAction): string | undefined {
  if (!action) return undefined
  if (action.type === 'trade_alloc' || action.type === 'sol_goods') return action.good_id
  return undefined
}

function alertIconId(kind: AlertKind): string {
  switch (kind) {
    case 'electricity_shortage':
      return 'electricity'
    case 'transportation_shortage':
      return 'transportation'
    case 'goods_shortage':
      return 'market'
    case 'needs_unmet':
      return 'starvation'
    case 'low_market_access':
      return 'market_access'
    case 'unfilled_education':
      return 'qualification'
    case 'unfilled_pops':
      return 'population'
    case 'underemployed':
      return 'unemployment'
  }
}

export function AlertsPane({
  result,
  icons,
  buildings = [],
  onApply,
}: {
  result: AlertsResult
  icons?: DefsIcons
  buildings?: BuildingEconomics[]
  onApply?: (delta: WorldDelta) => void
}) {
  if (result.alerts.length === 0) {
    return <p>No shortages detected in the current solve.</p>
  }
  return (
    <ul className="alerts-list">
      {result.alerts.map((alert) => (
        <li key={alert.id}>
          <AlertExpander alert={alert} icons={icons} buildings={buildings} onApply={onApply} />
        </li>
      ))}
    </ul>
  )
}

function AlertExpander({
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
          {alert.good_id && <GameIcon kind="good" id={alert.good_id} icons={icons} />}
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
            </li>
          )
        })}
      </ol>
    </details>
  )
}
