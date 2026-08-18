import { afterEach, describe, expect, it, vi } from 'vitest'
import { loadWasm, resetWasmCache, wasmPublicUrl } from './wasm'

describe('wasmPublicUrl', () => {
  it('is a root-relative path fetch can resolve, with a cache-bust query', () => {
    expect(wasmPublicUrl('vic3_wasm.js')).toMatch(/^\/(?:.*\/)?wasm\/vic3_wasm\.js\?v=.+/)
    expect(wasmPublicUrl('vic3_wasm_bg.wasm')).toMatch(
      /^\/(?:.*\/)?wasm\/vic3_wasm_bg\.wasm\?v=.+/,
    )
  })

  it('cannot be used as the base for new URL, which is how cache-busting broke init', () => {
    expect(() => new URL('vic3_wasm_bg.wasm', wasmPublicUrl('vic3_wasm.js'))).toThrow()
  })
})

describe('loadWasm', () => {
  afterEach(() => {
    resetWasmCache()
  })

  it('inits wasm-bindgen with the cache-busted binary URL, not a relative new URL()', async () => {
    const init = vi.fn(async () => {})
    const importModule = vi.fn(async () => ({ default: init }))
    await loadWasm({ importModule })
    expect(importModule).toHaveBeenCalledWith(wasmPublicUrl('vic3_wasm.js'))
    expect(init).toHaveBeenCalledTimes(1)
    expect(init).toHaveBeenCalledWith(wasmPublicUrl('vic3_wasm_bg.wasm'))
  })

  it('clears a failed load so a later call can retry', async () => {
    await expect(
      loadWasm({
        importModule: async () => {
          throw new Error('Failed to construct URL')
        },
      }),
    ).rejects.toThrow('Failed to construct URL')

    const init = vi.fn(async () => {})
    await loadWasm({
      importModule: async () => ({ default: init }),
    })
    expect(init).toHaveBeenCalled()
  })
})
