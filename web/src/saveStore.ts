import { openDB, type DBSchema } from 'idb'

/**
 * The last save a user dropped or picked, kept so a reload does not require
 * choosing the `.v3` again. Token maps ride along for binary saves.
 */
export interface StoredSave {
  id: 'current'
  name: string
  bytes: Uint8Array
  tokens?: Uint8Array
  tokens_name?: string
  saved_at: string
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
      database.createObjectStore('saves', { keyPath: 'id' })
    },
  })

function fileFromBytes(bytes: Uint8Array, name: string): File {
  return new File([bytes.slice().buffer as ArrayBuffer], name, {
    type: 'application/octet-stream',
  })
}

export async function storeSave(save: File, tokens?: File): Promise<void> {
  await (await db()).put('saves', {
    id: 'current',
    name: save.name,
    bytes: new Uint8Array(await save.arrayBuffer()),
    tokens: tokens ? new Uint8Array(await tokens.arrayBuffer()) : undefined,
    tokens_name: tokens?.name,
    saved_at: new Date().toISOString(),
  })
}

export async function loadStoredSave(): Promise<{ save: File; tokens?: File } | undefined> {
  const stored = await (await db()).get('saves', 'current')
  if (!stored) return undefined
  return {
    save: fileFromBytes(stored.bytes, stored.name),
    tokens: stored.tokens
      ? fileFromBytes(stored.tokens, stored.tokens_name ?? 'tokens.txt')
      : undefined,
  }
}

export async function clearStoredSave(): Promise<void> {
  await (await db()).clear('saves')
}
