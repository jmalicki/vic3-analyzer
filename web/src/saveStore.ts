import { openDB, type DBSchema } from 'idb'
import type {
  CurrentPointer,
  Origin,
  PricesResult,
  SaveSummary,
  Step,
  Timeline,
} from './types'

/**
 * Bump when the prices solver or `PricesResult` shape changes so a reload does
 * not paint a stale table. The save itself is still restored.
 */
export const PRICES_CACHE_VERSION = 2

export const SAVE_DB_NAME = 'vic3-analyzer-save'
export const SAVE_DB_VERSION = 2

/**
 * The last save a user dropped or picked, plus its last prices solve, so a
 * reload can show the campaign immediately without waiting on wasm.
 */
export interface StoredSave {
  id: 'current'
  name: string
  bytes?: Uint8Array
  blob?: Blob
  tokens?: Uint8Array | Blob
  tokens_name?: string
  saved_at: string
  summary?: SaveSummary
  prices?: PricesResult
  prices_cache_version?: number
}

export interface CommitStepInput {
  mutations: unknown[]
  summary?: SaveSummary
  prices?: PricesResult
  patchedBytes?: Uint8Array
  label?: string
}

interface SaveDb extends DBSchema {
  saves: {
    key: string
    value: StoredSave
  }
  origins: {
    key: string
    value: Origin
  }
  timelines: {
    key: string
    value: Timeline
    indexes: { 'by-origin': string }
  }
  steps: {
    key: string
    value: Step
    indexes: { 'by-timeline': string }
  }
  meta: {
    key: string
    value: CurrentPointer
  }
}

function newId(): string {
  return crypto.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(16).slice(2)}`
}

async function fingerprint(data: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', data.slice().buffer)
  return [...new Uint8Array(digest)].map((part) => part.toString(16).padStart(2, '0')).join('')
}

const db = () =>
  openDB<SaveDb>(SAVE_DB_NAME, SAVE_DB_VERSION, {
    async upgrade(database, oldVersion, _newVersion, transaction) {
      if (!database.objectStoreNames.contains('saves')) {
        database.createObjectStore('saves', { keyPath: 'id' })
      }
      if (!database.objectStoreNames.contains('origins')) {
        database.createObjectStore('origins', { keyPath: 'id' })
      }
      if (!database.objectStoreNames.contains('timelines')) {
        const store = database.createObjectStore('timelines', { keyPath: 'id' })
        store.createIndex('by-origin', 'origin_id')
      }
      if (!database.objectStoreNames.contains('steps')) {
        const store = database.createObjectStore('steps', { keyPath: 'id' })
        store.createIndex('by-timeline', 'timeline_id')
      }
      if (!database.objectStoreNames.contains('meta')) {
        database.createObjectStore('meta', { keyPath: 'id' })
      }

      if (oldVersion < 2 && database.objectStoreNames.contains('saves')) {
        const current = await transaction.objectStore('saves').get('current')
        if (!current) return
        const originId = newId()
        const timelineId = newId()
        const stepId = newId()
        const bytes = asBytes(current.bytes) ?? asBytes(current.blob)
        let hashed: string | undefined
        if (bytes) {
          try {
            hashed = await fingerprint(bytes)
          } catch {
            hashed = undefined
          }
        }
        await transaction.objectStore('origins').put({
          id: originId,
          name: current.name,
          bytes: current.bytes,
          blob: current.blob,
          tokens: current.tokens,
          tokens_name: current.tokens_name,
          fingerprint: hashed,
          saved_at: current.saved_at,
        })
        await transaction.objectStore('timelines').put({
          id: timelineId,
          origin_id: originId,
          label: 'Main',
          created_at: current.saved_at,
        })
        await transaction.objectStore('steps').put({
          id: stepId,
          timeline_id: timelineId,
          parent_step_id: null,
          mutations: [],
          summary: current.summary,
          prices: current.prices,
          prices_cache_version: current.prices_cache_version,
          created_at: current.saved_at,
        })
        await transaction.objectStore('meta').put({
          id: 'current',
          origin_id: originId,
          timeline_id: timelineId,
          step_id: stepId,
        })
      }
    },
  })

let writes: Promise<void> = Promise.resolve()

function enqueue(task: () => Promise<void>): Promise<void> {
  const run = writes.then(task, task)
  writes = run.catch(() => {})
  return run
}

export function persistErrorMessage(error: unknown): string {
  if (
    (error instanceof DOMException &&
      (error.name === 'QuotaExceededError' || error.name === 'DataCloneError')) ||
    (error instanceof Error && /quota|clone/i.test(error.message))
  ) {
    return 'This save could not be kept for the next visit. Analysis still runs now.'
  }
  return 'Save could not be kept in this browser; it lasts until reload.'
}

function asBytes(value: unknown): Uint8Array | undefined {
  if (!value) return undefined
  if (value instanceof Uint8Array) return value
  if (value instanceof ArrayBuffer) return new Uint8Array(value)
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength)
  }
  if (Array.isArray(value)) return new Uint8Array(value)
  if (typeof value === 'object') {
    const vals = Object.values(value as Record<string, unknown>)
    if (vals.length && vals.every((item) => typeof item === 'number')) {
      return new Uint8Array(vals as number[])
    }
  }
  return undefined
}

async function fileFromStored(value: unknown, name: string): Promise<File | undefined> {
  const bytes = asBytes(value)
  if (bytes) return new File([bytes], name)
  if (value && typeof value === 'object' && typeof (value as Blob).arrayBuffer === 'function') {
    return new File([await (value as Blob).arrayBuffer()], name)
  }
  return undefined
}

export function downloadName(originName: string, date: string, stepId: string): string {
  const stem = originName.replace(/\.[^.]+$/, '')
  return `${stem}_analyzer_${date}_${stepId}.v3`
}

export async function storeSave(save: File, tokens?: File): Promise<void> {
  await enqueue(async () => {
    const bytes = new Uint8Array(await save.arrayBuffer())
    const tokenBytes = tokens ? new Uint8Array(await tokens.arrayBuffer()) : undefined
    const now = new Date().toISOString()
    const originId = newId()
    const timelineId = newId()
    const stepId = newId()
    const database = await db()
    await database.put('origins', {
      id: originId,
      name: save.name,
      bytes,
      tokens: tokenBytes,
      tokens_name: tokens?.name,
      fingerprint: await fingerprint(bytes),
      saved_at: now,
    })
    await database.put('timelines', {
      id: timelineId,
      origin_id: originId,
      label: 'Main',
      created_at: now,
    })
    await database.put('steps', {
      id: stepId,
      timeline_id: timelineId,
      parent_step_id: null,
      mutations: [],
      created_at: now,
    })
    await database.put('meta', {
      id: 'current',
      origin_id: originId,
      timeline_id: timelineId,
      step_id: stepId,
    })
  })
}

export async function storeSaveAnalysis(
  summary: SaveSummary,
  prices: PricesResult,
): Promise<void> {
  await enqueue(async () => {
    const database = await db()
    const current = await database.get('meta', 'current')
    if (!current) return
    const step = await database.get('steps', current.step_id)
    if (!step) return
    await database.put('steps', {
      ...step,
      summary,
      prices,
      prices_cache_version: PRICES_CACHE_VERSION,
    })
  })
}

export async function loadStoredSave(): Promise<
  { save: File; tokens?: File; summary?: SaveSummary; prices?: PricesResult } | undefined
> {
  await writes
  const database = await db()
  const current = await database.get('meta', 'current')
  if (!current) return undefined
  const origin = await database.get('origins', current.origin_id)
  const step = await database.get('steps', current.step_id)
  if (!origin || !step) return undefined
  const save = await fileFromStored(origin.bytes ?? origin.blob, origin.name)
  if (!save) return undefined
  const tokens = origin.tokens
    ? await fileFromStored(origin.tokens, origin.tokens_name ?? 'tokens.txt')
    : undefined
  const pricesFresh = step.prices_cache_version === PRICES_CACHE_VERSION
  return {
    save,
    tokens,
    summary: step.summary,
    prices: pricesFresh ? step.prices : undefined,
  }
}

export async function clearStoredSave(): Promise<void> {
  await enqueue(async () => {
    const database = await db()
    await database.clear('saves')
    await database.clear('origins')
    await database.clear('timelines')
    await database.clear('steps')
    await database.clear('meta')
  })
}

export async function currentPointer(): Promise<CurrentPointer | undefined> {
  await writes
  return (await db()).get('meta', 'current')
}

export async function listOrigins(): Promise<Origin[]> {
  await writes
  return (await db()).getAll('origins')
}

export async function listTimelines(originId: string): Promise<Timeline[]> {
  await writes
  return (await db()).getAllFromIndex('timelines', 'by-origin', originId)
}

export async function listSteps(timelineId: string): Promise<Step[]> {
  await writes
  return (await db()).getAllFromIndex('steps', 'by-timeline', timelineId)
}

export async function checkout(originId: string, timelineId: string, stepId: string): Promise<void> {
  await enqueue(async () => {
    const database = await db()
    const origin = await database.get('origins', originId)
    const timeline = await database.get('timelines', timelineId)
    const step = await database.get('steps', stepId)
    if (!origin || !timeline || !step) return
    if (timeline.origin_id !== originId || step.timeline_id !== timelineId) return
    await database.put('meta', {
      id: 'current',
      origin_id: originId,
      timeline_id: timelineId,
      step_id: stepId,
    })
  })
}

export async function commitStep(input: CommitStepInput): Promise<Step> {
  let created: Step | undefined
  await enqueue(async () => {
    const database = await db()
    const current = await database.get('meta', 'current')
    if (!current) throw new Error('No current save to commit onto')
    const step: Step = {
      id: newId(),
      timeline_id: current.timeline_id,
      parent_step_id: current.step_id,
      mutations: input.mutations,
      summary: input.summary,
      prices: input.prices,
      prices_cache_version: input.prices !== undefined ? PRICES_CACHE_VERSION : undefined,
      patched_bytes: input.patchedBytes,
      created_at: new Date().toISOString(),
      label: input.label,
    }
    await database.put('steps', step)
    await database.put('meta', { ...current, step_id: step.id })
    created = step
  })
  if (!created) throw new Error('No current save to commit onto')
  return created
}

export async function savePoint(label: string): Promise<void> {
  await enqueue(async () => {
    const database = await db()
    const current = await database.get('meta', 'current')
    if (!current) return
    const step = await database.get('steps', current.step_id)
    if (!step) return
    await database.put('steps', { ...step, label })
  })
}
