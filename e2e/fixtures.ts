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
  balanced: 'mock_balanced.v3',
  barren: 'barren.v3',
  twoCountries: 'mock_two_countries.v3',
} as const

export type SaveKey = keyof typeof SAVES

/**
 * PR #76 multi-save matrix — each save must produce a distinct UI marker.
 * Primary save is `shortage` (mock_lumber + lumber camp + tool workshop).
 * `twoCountries` exercises player-scoped alerts / geography filters.
 */
export const SAVE_MARKERS: Record<
  SaveKey,
  { label: string; expectVisible: string[]; expectAbsent?: string[] }
> = {
  shortage: {
    label: 'shortage economy (lumber short vs tools)',
    expectVisible: ['mock_lumber', 'Mock Lumber Camp', 'Mock Tool Workshop'],
  },
  balanced: {
    label: 'balanced lumber-only economy',
    expectVisible: ['mock_lumber', 'Mock Lumber Camp'],
    expectAbsent: ['Mock Tool Workshop'],
  },
  barren: {
    label: 'barren GER stub',
    expectVisible: ['GER'],
    expectAbsent: ['Mock Lumber Camp', 'mock_lumber'],
  },
  twoCountries: {
    label: 'player MOCK + foreign RIVAL (Rivalia)',
    expectVisible: ['Home', 'mock_lumber'],
    expectAbsent: ['Rivalia'],
  },
}

/** Materialize *.v3 beside the plaintext fixtures (*.v3 is gitignored). */
export function prepareE2eSaves(): void {
  mkdirSync(e2eSavesDir, { recursive: true })
  const pairs: [string, string][] = [
    ['mock_shortage.txt', SAVES.shortage],
    ['mock_balanced.txt', SAVES.balanced],
    ['barren.txt', SAVES.barren],
    ['mock_two_countries.txt', SAVES.twoCountries],
  ]
  for (const [src, dest] of pairs) {
    copyFileSync(join(e2eSavesDir, src), join(e2eSavesDir, dest))
  }
}
