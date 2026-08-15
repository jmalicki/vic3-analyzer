import { openDB, type DBSchema } from 'idb'
import type { AnalysisRecord } from './types'

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
