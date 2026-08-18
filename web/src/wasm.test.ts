import { describe, expect, it } from 'vitest'
import { wasmPublicUrl } from './wasm'

describe('wasmPublicUrl', () => {
  it('is a root-relative path fetch can resolve, with a cache-bust query', () => {
    expect(wasmPublicUrl('vic3_wasm.js')).toMatch(/^\/(?:.*\/)?wasm\/vic3_wasm\.js\?v=.+/)
    expect(wasmPublicUrl('vic3_wasm_bg.wasm')).toMatch(
      /^\/(?:.*\/)?wasm\/vic3_wasm_bg\.wasm\?v=.+/,
    )
  })
})
