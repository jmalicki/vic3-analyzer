import { beforeEach, describe, expect, it } from 'vitest'
import { clearStoredSave, loadStoredSave, storeSave } from './saveStore'

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

  it('replacing a save drops the previous token map', async () => {
    await storeSave(new File(['a'], 'a.v3'), new File(['tok'], 'tokens.txt'))
    await storeSave(new File(['b'], 'b.v3'))
    const stored = await loadStoredSave()
    expect(stored?.save.name).toBe('b.v3')
    expect(stored?.tokens).toBeUndefined()
  })
})
