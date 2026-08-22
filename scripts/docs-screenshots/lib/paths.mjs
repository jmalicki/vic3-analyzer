import { existsSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))

/** Package root: scripts/docs-screenshots */
export const packageRoot = resolve(here, '..')

/** Repo root (vic3-analyzer worktree). */
export const repoRoot = resolve(packageRoot, '../..')

export const assetsDir = resolve(repoRoot, 'docs/assets')

export const webRoot = resolve(repoRoot, 'web')

export const desktopUiDir = resolve(repoRoot, 'crates/vic3-analyzer/ui')

export const saveFixture = resolve(
  repoRoot,
  'crates/vic3-load/tests/fixtures/plaintext.txt',
)

export const defsFixture = resolve(webRoot, 'fixtures/defs.postcard')

/** Locked filenames from docs/assets/README.md */
export const WEB_SHOTS = [
  'web-prices.png',
  'web-good-drilldown.png',
  'web-building-detail.png',
  'web-what-if.png',
  'web-buildings.png',
  'web-alerts.png',
  'web-gaps-war.png',
  'web-timeline-gdp.png',
  'web-archive.png',
  'web-defs-builder.png',
]

export const DESKTOP_SHOTS = [
  'desktop-dashboard.png',
  'desktop-saves.png',
  'desktop-query-shortages.png',
  'desktop-states.png',
  'desktop-prices.png',
  'desktop-timeline-gdp.png',
  'desktop-settings.png',
]

export const ALL_SHOTS = [...WEB_SHOTS, ...DESKTOP_SHOTS]

/**
 * Where generators write PNGs.
 * Default: docs/assets (local regen). CI sets DOCS_SCREENSHOTS_OUT to a scratch dir.
 */
export function outDir() {
  const fromEnv = process.env.DOCS_SCREENSHOTS_OUT?.trim()
  return fromEnv ? resolve(fromEnv) : assetsDir
}

export function requireDefsBlob() {
  if (!existsSync(defsFixture)) {
    throw new Error(
      `Missing ${defsFixture}. Run: (cd web && npm run build:defs) or npm run build`,
    )
  }
}

export function requireWasm() {
  const wasmJs = resolve(webRoot, 'public/wasm/vic3_wasm.js')
  if (!existsSync(wasmJs)) {
    throw new Error(
      `Missing wasm at ${wasmJs}. Run: (cd web && npm run build:wasm) or npm run build`,
    )
  }
}
