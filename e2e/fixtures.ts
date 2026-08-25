import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { copyFileSync, mkdirSync } from 'node:fs'

const here = dirname(fileURLToPath(import.meta.url))
export const repoRoot = join(here, '..')

export const mockGameDir = join(repoRoot, 'tests', 'fixtures', 'mock_game')
export const e2eSavesDir = join(repoRoot, 'tests', 'fixtures', 'e2e_saves')
/** Shared with `vic3-defs` blob guard (`BLOB_VERSION` postcard next to mock_game/). */
export const mockGameDefsPostcard = join(repoRoot, 'tests', 'fixtures', 'mock_game.defs.postcard')

/** Catalog / upload basenames (copied from *.txt in prepareE2eSaves). */
export const SAVES = {
  shortage: 'mock_shortage.v3',
} as const

export type SaveKey = keyof typeof SAVES

/** Materialize *.v3 beside the plaintext fixtures (*.v3 is gitignored). */
export function prepareE2eSaves(): void {
  mkdirSync(e2eSavesDir, { recursive: true })
  copyFileSync(join(e2eSavesDir, 'mock_shortage.txt'), join(e2eSavesDir, SAVES.shortage))
}
