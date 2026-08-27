import { openDB } from 'idb'
import { beforeEach, describe, expect, it } from 'vitest'
import {
  SAVE_DB_NAME,
  SAVE_DB_VERSION,
  checkout,
  clearStoredSave,
  commitStep,
  currentPointer,
  downloadName,
  listOrigins,
  listSteps,
  loadStoredSave,
  savePoint,
  storeSave,
  storeSaveAnalysis,
} from './saveStore'
import type { PricesResult, SaveSummary } from './types'

const summary: SaveSummary = {
  tag: 'PRU',
  version: '1.13.0',
  date: '1873.6.29',
}

const prices: PricesResult = {
  goods: [{ name: 'iron', base: 40, price: 42, buy: 1, sell: 1 }],
  residual: 0,
  status: 'converged',
  limitations: [],
}

describe('save store', () => {
  beforeEach(clearStoredSave)

  it('round-trips a save and optional token map', async () => {
    await storeSave(new File(['campaign'], 'prussia.v3'), new File(['0x1 foo'], 'tokens.txt'))
    const stored = await loadStoredSave()
    expect(stored?.save.name).toBe('prussia.v3')
    expect(await stored?.save.text()).toBe('campaign')
    expect(stored?.tokens?.name).toBe('tokens.txt')
    expect(await stored?.tokens?.text()).toBe('0x1 foo')
  })

  it('replacing a save drops the previous token map and analysis', async () => {
    await storeSave(new File(['a'], 'a.v3'), new File(['tok'], 'tokens.txt'))
    await storeSaveAnalysis(summary, prices)
    await storeSave(new File(['b'], 'b.v3'))
    const stored = await loadStoredSave()
    expect(stored?.save.name).toBe('b.v3')
    expect(stored?.tokens).toBeUndefined()
    expect(stored?.prices).toBeUndefined()
  })

  it('keeps the last prices solve next to the save', async () => {
    await storeSave(new File(['campaign'], 'prussia.v3'))
    await storeSaveAnalysis(summary, prices)
    const stored = await loadStoredSave()
    expect(stored?.summary).toEqual(summary)
    expect(stored?.prices?.goods[0]?.name).toBe('iron')
  })

  it('ignores a prices solve from another cache version', async () => {
    await storeSave(new File(['campaign'], 'prussia.v3'))
    await storeSaveAnalysis(summary, prices)
    const pointer = await currentPointer()
    const database = await openDB(SAVE_DB_NAME, SAVE_DB_VERSION)
    const step = await database.get('steps', pointer!.step_id)
    await database.put('steps', { ...step, prices_cache_version: 0 })
    const stored = await loadStoredSave()
    expect(stored?.save.name).toBe('prussia.v3')
    expect(stored?.summary).toEqual(summary)
    expect(stored?.prices).toBeUndefined()
  })

  it('keeps prior origins and restores the first file on checkout', async () => {
    await storeSave(new File(['first'], 'a.v3'))
    const first = await currentPointer()
    await storeSave(new File(['second'], 'b.v3'))
    expect((await listOrigins()).map((origin) => origin.name).sort()).toEqual(['a.v3', 'b.v3'])
    await checkout(first!.origin_id, first!.timeline_id, first!.step_id)
    const stored = await loadStoredSave()
    expect(stored?.save.name).toBe('a.v3')
    expect(await stored?.save.text()).toBe('first')
  })

  it('commitStep keeps origin bytes and records mutations on the new step', async () => {
    await storeSave(new File(['campaign'], 'prussia.v3'))
    const mutations = [{ kind: 'set', path: 'foo', value: 1 }]
    await commitStep({ mutations })
    const stored = await loadStoredSave()
    expect(await stored?.save.text()).toBe('campaign')
    const pointer = await currentPointer()
    const current = (await listSteps(pointer!.timeline_id)).find((step) => step.id === pointer!.step_id)
    expect(current?.mutations).toEqual(mutations)
    expect(current?.parent_step_id).toBeTruthy()
  })

  it('savePoint labels the current step', async () => {
    await storeSave(new File(['campaign'], 'prussia.v3'))
    await savePoint('before tax reform')
    const pointer = await currentPointer()
    const current = (await listSteps(pointer!.timeline_id)).find((step) => step.id === pointer!.step_id)
    expect(current?.label).toBe('before tax reform')
  })

  it('formats a patched download name from origin, date, and step', () => {
    expect(downloadName('prussia.v3', '1873.6.29', 'step-0')).toBe(
      'prussia_analyzer_1873.6.29_step-0.v3',
    )
  })
})
