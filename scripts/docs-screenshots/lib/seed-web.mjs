import { readFileSync } from 'node:fs'
import { defsFixture } from './paths.mjs'

/**
 * Seed fixture defs into IndexedDB so production vite preview (no demo defs)
 * unlocks analysis the same way a returning user with a stored blob would.
 *
 * @param {import('playwright').Page} page
 * @param {Uint8Array} [defsBytes]
 */
export async function seedDefsIndexedDb(page, defsBytes) {
  const bytes = defsBytes ?? new Uint8Array(readFileSync(defsFixture))
  const b64 = Buffer.from(bytes).toString('base64')
  const seed = page.evaluate(async (payload) => {
    const raw = atob(payload.b64)
    const bytes = new Uint8Array(raw.length)
    for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i)

    await new Promise((resolve, reject) => {
      const req = indexedDB.open('vic3-analyzer-defs', 1)
      req.onupgradeneeded = () => {
        const db = req.result
        if (!db.objectStoreNames.contains('defs')) {
          db.createObjectStore('defs', { keyPath: 'id' })
        }
      }
      req.onerror = () => reject(req.error ?? new Error('indexedDB open failed'))
      req.onsuccess = () => {
        const db = req.result
        const tx = db.transaction('defs', 'readwrite')
        tx.objectStore('defs').put({
          id: 'current',
          name: 'defs.postcard',
          bytes,
          saved_at: new Date().toISOString(),
        })
        tx.oncomplete = () => {
          db.close()
          resolve()
        }
        tx.onerror = () => reject(tx.error ?? new Error('indexedDB write failed'))
      }
    })
  }, { b64 })

  await Promise.race([
    seed,
    new Promise((_, reject) =>
      setTimeout(() => reject(new Error('seedDefsIndexedDb timed out after 30s')), 30_000),
    ),
  ])
}
