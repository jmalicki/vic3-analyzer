import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  formatAnalysisEngineLoadError,
  loadWasm,
  resetWasmCache,
  wasmPublicUrl,
} from './wasm'

describe('wasmPublicUrl', () => {
  it('is a root-relative path fetch can resolve, with a cache-bust query', () => {
    expect(wasmPublicUrl('vic3_wasm.js')).toMatch(/^\/(?:.*\/)?wasm\/vic3_wasm\.js\?v=.+/)
    expect(wasmPublicUrl('vic3_wasm_bg.wasm')).toMatch(
      /^\/(?:.*\/)?wasm\/vic3_wasm_bg\.wasm\?v=.+/,
    )
  })

  it('forces relative Vite bases to `/` so import() does not resolve under /assets/', () => {
    const previous = import.meta.env.BASE_URL
    import.meta.env.BASE_URL = './'
    try {
      expect(wasmPublicUrl('vic3_wasm.js')).toMatch(/^\/wasm\/vic3_wasm\.js\?v=.+/)
    } finally {
      import.meta.env.BASE_URL = previous
    }
  })

  it('cannot be used as the base for new URL, which is how cache-busting broke init', () => {
    expect(() => new URL('vic3_wasm_bg.wasm', wasmPublicUrl('vic3_wasm.js'))).toThrow()
  })
})

describe('formatAnalysisEngineLoadError', () => {
  it('rewrites Chromium MIME failures instead of echoing them', () => {
    const message = formatAnalysisEngineLoadError(
      new Error("'text/html' is not a valid JavaScript MIME type"),
      '/wasm/vic3_wasm.js?v=deadbeef',
    )
    expect(message).toMatch(/web page instead of JavaScript/)
    expect(message).toMatch(/Tried: \/wasm\/vic3_wasm\.js/)
    expect(message).not.toMatch(/MIME type/)
  })

  it('does not double-prefix an already formatted message', () => {
    const once = formatAnalysisEngineLoadError(new Error("Failed to construct 'URL'"))
    expect(formatAnalysisEngineLoadError(new Error(once))).toBe(once)
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
    ).rejects.toThrow(/Could not load the analysis engine.*Failed to construct URL/)

    const init = vi.fn(async () => {})
    await loadWasm({
      importModule: async () => ({ default: init }),
    })
    expect(init).toHaveBeenCalled()
  })

  it('surfaces MIME failures with the attempted glue URL', async () => {
    await expect(
      loadWasm({
        moduleUrl: '/wasm/missing.js',
        importModule: async () => {
          throw new Error("'text/html' is not a valid JavaScript MIME type")
        },
      }),
    ).rejects.toThrow(/Tried: \/wasm\/missing\.js/)
  })
})
