import { openDB, type DBSchema } from 'idb'
import type {
  ActionDiff,
  AnalysisRecord,
  CompareResult,
  GapSimpleSubgoal,
  GapDiff,
  GapsResult,
  PlanResult,
  PricesResult,
} from './types'

interface ArchiveDb extends DBSchema {
  analyses: {
    key: string
    value: AnalysisRecord
    indexes: {
      'by-created-at': string
      'by-fingerprint': string
    }
  }
}

const db = () =>
  openDB<ArchiveDb>('vic3-analyzer', 1, {
    upgrade(database) {
      const store = database.createObjectStore('analyses', { keyPath: 'id' })
      store.createIndex('by-created-at', 'created_at')
      store.createIndex('by-fingerprint', 'fingerprint')
    },
  })

export async function saveAnalysis(record: AnalysisRecord): Promise<void> {
  await (await db()).put('analyses', record)
}

export async function listAnalyses(): Promise<AnalysisRecord[]> {
  const records = await (await db()).getAllFromIndex('analyses', 'by-created-at')
  return records.reverse()
}

export async function getAnalysis(id: string): Promise<AnalysisRecord | undefined> {
  return (await db()).get('analyses', id)
}

export async function clearAnalyses(): Promise<void> {
  await (await db()).clear('analyses')
}

export function serializeAnalysis(record: AnalysisRecord): string {
  return JSON.stringify(
    record,
    (_key, value: unknown) => (value instanceof Uint8Array ? Array.from(value) : value),
    2,
  )
}

export function parseAnalysis(json: string): AnalysisRecord {
  const value = JSON.parse(json) as AnalysisRecord
  if (!value.id || !value.created_at || !value.kind || !value.fingerprint || !value.result) {
    throw new Error('File is not an AnalysisRecord.')
  }
  if (value.blob?.save) {
    value.blob.save = toBytes(value.blob.save)
    if (value.blob.tokens) value.blob.tokens = toBytes(value.blob.tokens)
  }
  return value
}

function toBytes(value: Uint8Array): Uint8Array {
  if (value instanceof Uint8Array) return value
  if (Array.isArray(value)) return new Uint8Array(value)
  return new Uint8Array(Object.values(value as unknown as Record<string, number>))
}

function goodScriptKey(good: {
  name?: string
  good_name?: string
  id?: string
}): string | undefined {
  return good.name ?? good.good_name ?? good.id
}

export function compareAnalyses(left: AnalysisRecord, right: AnalysisRecord): CompareResult {
  const comparison: CompareResult = {
    left: left.id,
    right: right.id,
    same_fingerprint: left.fingerprint === right.fingerprint,
  }

  if (left.kind === 'plan' && right.kind === 'plan') {
    const leftPlan = left.result as PlanResult
    const rightPlan = right.result as PlanResult
    comparison.day_cost_delta = rightPlan.day_cost - leftPlan.day_cost
    const actions = alignActions(leftPlan.actions, rightPlan.actions)
    if (actions.length) comparison.actions = actions
  } else if (
    (left.kind === 'prices' || left.kind === 'what_if') &&
    (right.kind === 'prices' || right.kind === 'what_if')
  ) {
    const rightGoods = new Map(
      (right.result as PricesResult).goods.map((good) => [goodScriptKey(good), good]),
    )
    const prices = (left.result as PricesResult).goods.flatMap((good) => {
      const key = goodScriptKey(good)
      const other = key ? rightGoods.get(key) : undefined
      const delta = other ? other.price - good.price : 0
      return other && delta !== 0 ? [{ good: key!, delta }] : []
    })
    if (prices.length) comparison.prices = prices
  } else if (
    left.kind === 'gaps' &&
    right.kind === 'gaps' &&
    serializeAnalysis(left) !== serializeAnalysis(right)
  ) {
    const leftGaps = (left.result as GapsResult).gaps
    const rightGaps = (right.result as GapsResult).gaps
    const gaps: GapDiff[] = leftGaps.map((subgoal) => ({
      simple_subgoal: subgoal,
      status: includesSimpleSubgoal(rightGaps, subgoal) ? 'still_failing' : 'cleared',
    }))
    for (const subgoal of rightGaps) {
      if (!includesSimpleSubgoal(leftGaps, subgoal))
        gaps.push({ simple_subgoal: subgoal, status: 'newly_failing' })
    }
    if (gaps.length) comparison.gaps = gaps
  }

  return comparison
}

function includesSimpleSubgoal(
  items: GapSimpleSubgoal[],
  candidate: GapSimpleSubgoal,
): boolean {
  const encoded = JSON.stringify(candidate)
  return items.some((item) => JSON.stringify(item) === encoded)
}

function alignActions(left: PlanResult['actions'], right: PlanResult['actions']): ActionDiff[] {
  const lengths = Array.from({ length: left.length + 1 }, () =>
    Array<number>(right.length + 1).fill(0),
  )
  for (let leftIndex = left.length - 1; leftIndex >= 0; leftIndex -= 1) {
    for (let rightIndex = right.length - 1; rightIndex >= 0; rightIndex -= 1) {
      lengths[leftIndex][rightIndex] =
        JSON.stringify(left[leftIndex].action) === JSON.stringify(right[rightIndex].action)
          ? lengths[leftIndex + 1][rightIndex + 1] + 1
          : Math.max(lengths[leftIndex + 1][rightIndex], lengths[leftIndex][rightIndex + 1])
    }
  }

  const differences: ActionDiff[] = []
  let leftIndex = 0
  let rightIndex = 0
  while (leftIndex < left.length || rightIndex < right.length) {
    if (
      leftIndex < left.length &&
      rightIndex < right.length &&
      JSON.stringify(left[leftIndex].action) === JSON.stringify(right[rightIndex].action)
    ) {
      if (JSON.stringify(left[leftIndex]) !== JSON.stringify(right[rightIndex])) {
        differences.push({ left: left[leftIndex], right: right[rightIndex] })
      }
      leftIndex += 1
      rightIndex += 1
    } else if (
      rightIndex < right.length &&
      (leftIndex === left.length ||
        lengths[leftIndex][rightIndex + 1] >= lengths[leftIndex + 1][rightIndex])
    ) {
      differences.push({ right: right[rightIndex] })
      rightIndex += 1
    } else {
      differences.push({ left: left[leftIndex] })
      leftIndex += 1
    }
  }
  return differences
}
