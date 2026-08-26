import { openDB, type DBSchema } from 'idb'

/**
 * The definitions blob a user built or picked, kept so a reload does not drop
 * back to having no definitions (or, in development, to the demo fixture).
 */
export interface StoredDefs {
  id: 'current'
  name: string
  bytes: Uint8Array
  saved_at: string
}

interface DefsDb extends DBSchema {
  defs: {
    key: string
    value: StoredDefs
  }
}

const db = () =>
  openDB<DefsDb>('vic3-analyzer-defs', 1, {
    upgrade(database) {
      database.createObjectStore('defs', { keyPath: 'id' })
    },
  })

export async function storeDefs(file: File): Promise<void> {
  await (await db()).put('defs', { id: 'current', name: file.name,
    bytes: new Uint8Array(await file.arrayBuffer()),
    saved_at: new Date().toISOString(),
  })
}

export async function loadStoredDefs(): Promise<File | undefined> {
  const stored = await (await db()).get('defs', 'current')
  if (!stored) return undefined
  return new File([stored.bytes.slice().buffer as ArrayBuffer], stored.name, {
    type: 'application/octet-stream',
  })
}

export async function clearStoredDefs(): Promise<void> {
  await (await db()).clear('defs')
}
