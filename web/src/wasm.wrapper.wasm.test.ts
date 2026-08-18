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
    defs = readBytes(webRoot, 'fixtures/defs.postcard')
  }, 60_000)

  it('parse_save returns tag and date from the plaintext fixture', async () => {
    const summary = JSON.parse(await api.parse_save(save))
    expect(summary.tag).toBe('GER')
    expect(summary.country_id).toBe(16777216)
    expect(summary.market_id).toBe(1)
    expect(summary.date).toBe('1836.1.1')
    expect(summary.version).toBe('1.9.0')
    expect(summary.buildings).toContain('building_rye_farm')
  })

  it('builds a defs blob from an in-memory browser manifest', async () => {
    const goods = new TextEncoder().encode('grain = { cost = 20 }')
    const manifest = JSON.stringify([
      {
        path: 'Victoria 3/game/common/goods/goods.txt',
        offset: 0,
        length: goods.length,
      },
    ])
    const blob = await api.build_defs_blob(manifest, goods)
    expect(blob).toBeInstanceOf(Uint8Array)
    expect(blob.length).toBeGreaterThan(0)

    const summary = JSON.parse(await api.defs_summary(blob))
    expect(summary.goods).toBe(1)
  })

  it('streams batches into the same blob a single call produces', async () => {
    const sources = [
      { path: 'game/common/goods/00_goods.txt', text: 'grain = { cost = 20 }' },
      { path: 'game/common/goods/01_goods.txt', text: 'wood = { cost = 30 }' },
      { path: 'game/localization/english/goods_l_english.yml', text: 'grain:0 "Grain"' },
    ]
    const pack = (files: typeof sources) => {
      const encoded = files.map((file) => ({
        path: file.path,
        bytes: new TextEncoder().encode(file.text),
      }))
      const contents = new Uint8Array(
        encoded.reduce((total, file) => total + file.bytes.length, 0),
      )
      let offset = 0
      const manifest = encoded.map((file) => {
        contents.set(file.bytes, offset)
        const entry = { path: file.path, offset, length: file.bytes.length }
        offset += file.bytes.length
        return entry
      })
      return { manifestJson: JSON.stringify(manifest), contents }
    }

    const builder = new api.DefsBlobBuilder()
    // One file per batch, the way a browser hands over what it has just read.
    for (const source of sources) {
      const batch = pack([source])
      await builder.addBatch(batch.manifestJson, batch.contents)
    }
    const streamed = await builder.finish()

    const whole = pack(sources)
    const oneShot = await api.build_defs_blob(whole.manifestJson, whole.contents)
    expect(Array.from(streamed)).toEqual(Array.from(oneShot))

    const summary = JSON.parse(await api.defs_summary(streamed))
    expect(summary.goods).toBe(2)
    expect(summary.labels).toBe(1)
  })

  it('classifies source paths with the Rust-owned allowlist', () => {
    expect(api.classify_defs_path('game/common/goods/00_goods.txt', false)).toBe('read')
    expect(api.classify_defs_path('game/gfx/models', true)).toBe('prune')
    expect(api.classify_defs_path('game/gfx/interface/icons/goods_icons/grain.dds', false)).toBe(
      'read',
    )
    expect(
      api.classify_defs_path('game/gfx/interface/icons/building_icons/building_rye_farm.dds', false),
    ).toBe('read')
    expect(
      api.classify_defs_path('game/gfx/interface/icons/pops_icons/academics.dds', false),
    ).toBe('read')
    expect(
      api.classify_defs_path(
        'game/gfx/interface/icons/ships/ship_types/silhouette_frigate.dds',
        false,
      ),
    ).toBe('read')
    expect(api.classify_defs_path('game/gfx/interface/icons/country_icons', true)).toBe('prune')
  })

  /**
   * A blob left in IndexedDB by an older build has a payload the current
   * `GameDefs` cannot read, so the version must be reported rather than
   * whatever the stale payload trips over first.
   */
  it('rejects a stale blob with a version mismatch, not a decode error', async () => {
    const stale = new Uint8Array(defs)
    stale[0] = 1

    // The export may throw synchronously or reject; either must carry the version.
    await expect(Promise.resolve().then(() => api.defs_summary(stale))).rejects.toThrow(
      /defs blob version 1 is not supported \(expected \d+\)/,
    )
  })

  it('decodes the fixture DDS icon into a PNG data URL', async () => {
    const icons = JSON.parse(await api.defs_icons(defs))
    expect(icons.grain).toMatch(/^data:image\/png;base64,iVBOR/)
    expect(icons.goods.grain).toBe(icons.grain)
    expect(icons.extra['building:building_rye_farm']).toMatch(/^data:image\/png;base64,iVBOR/)
  })

  it('loads and reuses a worker-style analysis session', async () => {
    await api.clear_analysis()
    await expect(Promise.resolve().then(() => api.loaded_prices())).rejects.toThrow(
      'no analysis is loaded',
    )

    const loaded = JSON.parse(await api.load_analysis(save, undefined, defs, '{}'))
    expect(loaded.summary.tag).toBe('GER')
    expect(loaded.prices.goods.length).toBeGreaterThan(0)

    const cached = JSON.parse(await api.loaded_prices())
    expect(cached).toEqual(loaded.prices)
    const changed = JSON.parse(
      await api.loaded_what_if(
        JSON.stringify({ building: 'building_rye_farm', extra_levels: 1 }),
      ),
    )
    expect(changed.goods.length).toBeGreaterThan(0)
  })

  it('prices returns residual and limitations', async () => {
    const result = JSON.parse(await api.prices(save, undefined, defs, '{}'))
    expect(typeof result.residual).toBe('number')
    expect(Number.isFinite(result.residual)).toBe(true)
    expect(result.limitations.length).toBeGreaterThan(0)
    expect(result.goods.length).toBeGreaterThan(0)
    expect(result.scope).toBe('whole_save_synthetic')
    expect(result.states).toEqual(
      expect.arrayContaining([expect.objectContaining({ region_id: 'STATE_BRANDENBURG' })]),
    )
    expect(result.buildings).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          state_id: 1,
          production_method_ids: ['pm_simple_forestry'],
        }),
      ]),
    )
    expect(result.inputs.goods_with_orders).toBeGreaterThan(0)
    expect(
      result.goods.some((good: { base: number; price: number }) => good.price !== good.base),
    ).toBe(true)
    const wood = result.goods.find((good: { id: string }) => good.id === 'wood')
    expect(wood?.name).toBe('Wood')
    expect(wood?.sell).toBe(40)
    expect(wood?.price).toBeLessThan(wood?.base)
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
