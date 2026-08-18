import { Modal } from './Modal'
import type {
  BuildingEconomics,
  ExtraLevelsDelta,
  MitigationAction,
  PricesResult,
  SavePatch,
  Step,
  WorldDelta,
} from './types'

export function mutationLines(delta: WorldDelta): string[] {
  const lines: string[] = []
  for (const extra of delta.extra_levels ?? []) {
    lines.push(`+${extra.extra_levels} levels on ${extraTarget(extra)}`)
  }
  for (const method of delta.production_methods ?? []) {
    const names = method.methods.join(', ') || '(none)'
    lines.push(`Set production methods on building #${method.building_id}: ${names}`)
  }
  for (const subsidy of delta.subsidize ?? []) {
    const verb = subsidy.enabled ? 'Enable' : 'Disable'
    lines.push(`${verb} subsidy on building #${subsidy.building_id}`)
  }
  return lines
}

function extraTarget(extra: ExtraLevelsDelta): string {
  if (extra.building_id != null) return `building #${extra.building_id}`
  if (extra.building) return extra.building.replaceAll('_', ' ')
  return 'building'
}

export function matchingBuildingIds(
  buildings: BuildingEconomics[],
  typeId: string,
  stateId?: number,
): number[] {
  return buildings
    .filter(
      (building) =>
        building.type_id === typeId && (stateId == null || building.state_id === stateId),
    )
    .map((building) => building.id)
}

function findProducer(
  buildings: BuildingEconomics[],
  goodId: string,
  stateId?: number,
): BuildingEconomics | undefined {
  const produces = (building: BuildingEconomics) =>
    building.outputs.some((flow) => flow.good_id === goodId)
  if (stateId != null) {
    const local = buildings.find((building) => building.state_id === stateId && produces(building))
    if (local) return local
  }
  return buildings.find(produces)
}

/** Map an alert mitigation to a WorldDelta, or undefined when Apply must stay disabled. */
export function deltaForMitigation(
  action: MitigationAction | undefined,
  buildings: BuildingEconomics[] = [],
): WorldDelta | undefined {
  if (!action) return undefined
  switch (action.type) {
    case 'trade_alloc':
    case 'subsidize':
      return undefined
    case 'pm': {
      if (!action.building_id || !action.production_method) return undefined
      const methods =
        action.methods && action.methods.length > 0 ? action.methods : [action.production_method]
      return {
        production_methods: [{ building_id: action.building_id, methods }],
      }
    }
    case 'build': {
      if (!action.building) return undefined
      const extra = action.extra_levels ?? 1
      const ids = matchingBuildingIds(buildings, action.building, action.state_id)
      const fallback = ids.length ? ids : matchingBuildingIds(buildings, action.building)
      if (!fallback.length) return undefined
      return {
        extra_levels: fallback.map((building_id) => ({ building_id, extra_levels: extra })),
      }
    }
    case 'feeder_job': {
      if (!action.building) return undefined
      const ids = matchingBuildingIds(buildings, action.building, action.state_id)
      const fallback = ids.length ? ids : matchingBuildingIds(buildings, action.building)
      if (!fallback.length) return undefined
      return {
        extra_levels: fallback.map((building_id) => ({ building_id, extra_levels: 1 })),
      }
    }
    case 'sol_goods': {
      if (!action.good_id) return undefined
      const producer = findProducer(buildings, action.good_id, action.state_id)
      if (!producer) return undefined
      return {
        extra_levels: [{ building_id: producer.id, extra_levels: 1 }],
      }
    }
  }
}

export function worldDeltaToSavePatch(
  delta: WorldDelta,
  buildings: BuildingEconomics[] = [],
): SavePatch | undefined {
  const extra_levels: Array<{ building_id: number; extra_levels: number }> = []
  for (const extra of delta.extra_levels ?? []) {
    if (extra.building_id != null) {
      extra_levels.push({ building_id: extra.building_id, extra_levels: extra.extra_levels })
      continue
    }
    if (!extra.building) return undefined
    const ids = matchingBuildingIds(buildings, extra.building)
    if (!ids.length) return undefined
    for (const building_id of ids) {
      extra_levels.push({ building_id, extra_levels: extra.extra_levels })
    }
  }
  const production_methods = [...(delta.production_methods ?? [])]
  if (production_methods.some((item) => !item.building_id || !item.methods.length)) {
    return undefined
  }
  if (!extra_levels.length && !production_methods.length) return undefined
  return { extra_levels, production_methods }
}

export function mergeDeltas(deltas: WorldDelta[]): WorldDelta {
  return {
    extra_levels: deltas.flatMap((delta) => delta.extra_levels ?? []),
    production_methods: deltas.flatMap((delta) => delta.production_methods ?? []),
    subsidize: deltas.flatMap((delta) => delta.subsidize ?? []),
  }
}

function asWorldDelta(value: unknown): WorldDelta | undefined {
  if (!value || typeof value !== 'object') return undefined
  return value as WorldDelta
}

/** Mutations from origin through `currentId`, oldest first. */
export function deltasFromSteps(steps: Step[], currentId?: string): WorldDelta[] {
  if (!currentId) return []
  const byId = new Map(steps.map((step) => [step.id, step]))
  const chain: WorldDelta[][] = []
  const seen = new Set<string>()
  let id: string | undefined | null = currentId
  while (id && !seen.has(id)) {
    seen.add(id)
    const step = byId.get(id)
    if (!step) break
    chain.unshift((step.mutations ?? []).map(asWorldDelta).filter((delta): delta is WorldDelta => Boolean(delta)))
    id = step.parent_step_id
  }
  return chain.flat()
}

function pickGoods(current: PricesResult, preview: PricesResult) {
  const afterById = new Map(preview.goods.map((good) => [good.id, good]))
  const rows = current.goods.flatMap((good) => {
    const after = afterById.get(good.id)
    if (!after) return []
    return [
      {
        id: good.id,
        name: good.name ?? good.id.replaceAll('_', ' '),
        before: good.price,
        after: after.price,
      },
    ]
  })
  const changed = rows.filter((row) => row.before !== row.after)
  return (changed.length ? changed : rows).slice(0, 6)
}

export function ConfirmApply({
  delta,
  current,
  preview,
  error,
  busy = false,
  onConfirm,
  onCancel,
}: {
  delta: WorldDelta
  current: PricesResult
  preview: PricesResult
  error?: string
  busy?: boolean
  onConfirm: () => void
  onCancel: () => void
}) {
  const goods = pickGoods(current, preview)
  return (
    <Modal title="Confirm apply" locked={busy} onClose={onCancel}>
      <div className="confirm-apply">
        <h4>Mutations</h4>
        <ul className="confirm-mutations">
          {mutationLines(delta).map((line) => (
            <li key={line}>{line}</li>
          ))}
        </ul>
        <h4>Before / after</h4>
        <dl className="confirm-compare">
          <div>
            <dt>Residual</dt>
            <dd>
              <span>{current.residual}</span>
              <span aria-hidden="true"> → </span>
              <span>{preview.residual}</span>
            </dd>
          </div>
          {goods.map((good) => (
            <div key={good.id}>
              <dt>{good.name}</dt>
              <dd>
                <span>{good.before.toFixed(2)}</span>
                <span aria-hidden="true"> → </span>
                <span>{good.after.toFixed(2)}</span>
              </dd>
            </div>
          ))}
        </dl>
        {error && <p role="alert">{error}</p>}
        <div className="confirm-actions">
          <button type="button" disabled={busy} onClick={onConfirm}>
            Confirm
          </button>
          <button type="button" className="secondary" disabled={busy} onClick={onCancel}>
            Cancel
          </button>
        </div>
      </div>
    </Modal>
  )
}
