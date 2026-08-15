import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { beforeAll, describe, expect, it } from 'vitest'
import { loadWasm, resetWasmCache, type WasmApi } from './wasm'

const here = dirname(fileURLToPath(import.meta.url))
const repoRoot = resolve(here, '../..')
const webRoot = resolve(here, '..')

function readBytes(...parts: string[]): Uint8Array {
  return new Uint8Array(readFileSync(resolve(...parts)))
}

describe('wasm wrapper (real wasm-pack build)', () => {
  let api: WasmApi
  let save: Uint8Array
  let barren: Uint8Array
  let defs: Uint8Array

  beforeAll(async () => {
    resetWasmCache()
    const wasmDir = resolve(webRoot, 'public/wasm')
    const moduleUrl = pathToFileURL(resolve(wasmDir, 'vic3_wasm.js')).href
    const moduleOrPath = readFileSync(resolve(wasmDir, 'vic3_wasm_bg.wasm'))
    api = await loadWasm({ moduleUrl, moduleOrPath })
    save = readBytes(repoRoot, 'crates/vic3-load/tests/fixtures/plaintext.txt')
    barren = readBytes(repoRoot, 'crates/vic3-cli/tests/fixtures/barren.txt')
    defs = readBytes(webRoot, 'public/defs.postcard')
  }, 60_000)

  it('parse_save returns tag and date from the plaintext fixture', async () => {
    const summary = JSON.parse(await api.parse_save(save))
    expect(summary.tag).toBe('GER')
    expect(summary.date).toBe('1836.1.1')
    expect(summary.version).toBe('1.9.0')
    expect(summary.buildings).toContain('building_rye_farm')
  })

  it('prices returns residual and limitations', async () => {
    const result = JSON.parse(await api.prices(save, undefined, defs, '{}'))
    expect(typeof result.residual).toBe('number')
    expect(Number.isFinite(result.residual)).toBe(true)
    expect(result.limitations.length).toBeGreaterThan(0)
    expect(result.goods.length).toBeGreaterThan(0)
  })

  it('what_if returns residual and limitations', async () => {
    const result = JSON.parse(
      await api.what_if(
        save,
        undefined,
        defs,
        '{}',
        JSON.stringify({ building: 'building_rye_farm', extra_levels: 5 }),
      ),
    )
    expect(typeof result.residual).toBe('number')
    expect(result.limitations.length).toBeGreaterThan(0)
  })

  it('plan with research(tech=…) returns day cost and actions', async () => {
    const result = JSON.parse(
      await api.plan(
        save,
        undefined,
        defs,
        '{}',
        JSON.stringify({
          goal: 'research(tech=nitroglycerin)',
          max_days: 1000,
          label: 'rush',
        }),
      ),
    )
    expect(result.day_cost).toBe(365)
    expect(result.actions.length).toBe(2)
  })

  it('gaps returns unsatisfied atoms on the barren fixture', async () => {
    const result = JSON.parse(
      await api.gaps(
        barren,
        undefined,
        defs,
        '{}',
        'declare-war(tag=FRA, wargoal=conquer_state, state=alsace)',
      ),
    )
    expect(result.satisfied).toBe(false)
    expect(result.gaps).toHaveLength(4)
    expect(result.limitations.length).toBeGreaterThan(0)
  })

  it('schemas are non-empty and describe required fields', () => {
    const whatIf = JSON.parse(api.what_if_schema())
    expect(whatIf.properties.building).toBeTruthy()
    expect(whatIf.properties.extra_levels).toBeTruthy()
    expect(whatIf.required).toEqual(expect.arrayContaining(['building', 'extra_levels']))

    const prices = JSON.parse(api.prices_schema())
    expect(prices.properties.residual).toBeTruthy()
    expect(prices.properties.limitations).toBeTruthy()
    expect(prices.properties.goods).toBeTruthy()
  })
})
