import { openDB } from 'idb'
import { beforeEach, describe, expect, it } from 'vitest'
import { clearStoredSave, loadStoredSave, storeSave, storeSaveAnalysis } from './saveStore'
import type { PricesResult, SaveSummary } from './types'

const summary: SaveSummary = {
  tag: 'PRU',
  version: '1.13.0',
  date: '1873.6.29',
}

const prices: PricesResult = {
  goods: [{ id: 'iron', base: 40, price: 42, buy: 1, sell: 1 }],
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
    expect(stored?.prices?.goods[0]?.id).toBe('iron')
  })

  it('ignores a prices solve from another cache version', async () => {
    await storeSave(new File(['campaign'], 'prussia.v3'))
    await storeSaveAnalysis(summary, prices)
    const database = await openDB('vic3-analyzer-save', 1)
    const current = await database.get('saves', 'current')
    await database.put('saves', { ...current, prices_cache_version: 0 })
    const stored = await loadStoredSave()
    expect(stored?.save.name).toBe('prussia.v3')
    expect(stored?.summary).toEqual(summary)
    expect(stored?.prices).toBeUndefined()
  })
})
