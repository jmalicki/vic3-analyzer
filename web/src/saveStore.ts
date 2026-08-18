import { openDB, type DBSchema } from 'idb'
import type { PricesResult, SaveSummary } from './types'

/**
 * Bump when the prices solver or `PricesResult` shape changes so a reload does
 * not paint a stale table. The save itself is still restored.
 */
export const PRICES_CACHE_VERSION = 1

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

interface SaveDb extends DBSchema {
  saves: {
    key: string
    value: StoredSave
  }
}

const db = () =>
  openDB<SaveDb>('vic3-analyzer-save', 1, {
    upgrade(database) {
      if (!database.objectStoreNames.contains('saves')) {
        database.createObjectStore('saves', { keyPath: 'id' })
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

export async function storeSave(save: File, tokens?: File): Promise<void> {
  await enqueue(async () => {
    await (await db()).put('saves', {
      id: 'current',
      name: save.name,
      bytes: new Uint8Array(await save.arrayBuffer()),
      tokens: tokens ? new Uint8Array(await tokens.arrayBuffer()) : undefined,
      tokens_name: tokens?.name,
      saved_at: new Date().toISOString(),
    })
  })
}

export async function storeSaveAnalysis(
  summary: SaveSummary,
  prices: PricesResult,
): Promise<void> {
  await enqueue(async () => {
    const current = await (await db()).get('saves', 'current')
    if (!current) return
    await (await db()).put('saves', {
      ...current,
      summary,
      prices,
      prices_cache_version: PRICES_CACHE_VERSION,
    })
  })
}

export async function loadStoredSave(): Promise<
  { save: File; tokens?: File; summary?: SaveSummary; prices?: PricesResult } | undefined
> {
  const stored = await (await db()).get('saves', 'current')
  if (!stored) return undefined
  const save = await fileFromStored(stored.bytes ?? stored.blob, stored.name)
  if (!save) return undefined
  const tokens = stored.tokens
    ? await fileFromStored(stored.tokens, stored.tokens_name ?? 'tokens.txt')
    : undefined
  const pricesFresh = stored.prices_cache_version === PRICES_CACHE_VERSION
  return {
    save,
    tokens,
    summary: stored.summary,
    prices: pricesFresh ? stored.prices : undefined,
  }
}

export async function clearStoredSave(): Promise<void> {
  await enqueue(async () => {
    await (await db()).clear('saves')
  })
}
