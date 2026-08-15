import type { JsonSchema } from './types'

export interface WasmApi {
  parse_save(save: Uint8Array, tokens?: Uint8Array): string | Promise<string>
  prices(
    save: Uint8Array,
    tokens: Uint8Array | undefined,
    defs: Uint8Array,
    solveOptsJson: string,
  ): string | Promise<string>
  what_if(
    save: Uint8Array,
    tokens: Uint8Array | undefined,
    defs: Uint8Array,
    solveOptsJson: string,
    whatIfOptsJson: string,
  ): string | Promise<string>
  // The Rust wasm export is scheduled for P10/follow-up work. Keeping it
  // optional lets the UI use a mocked implementation until that lands.
  gaps?(
    save: Uint8Array,
    tokens: Uint8Array | undefined,
    defs: Uint8Array,
    solveOptsJson: string,
    goal: string,
  ): string | Promise<string>
  what_if_schema(): string
  prices_schema(): string
}

let cached: Promise<WasmApi> | undefined

export function loadWasm(): Promise<WasmApi> {
  const wasmPath = '/wasm/vic3_wasm.js'
  cached ??= import(/* @vite-ignore */ wasmPath).then(async (module) => {
    if (typeof module.default === 'function') await module.default()
    return module as unknown as WasmApi
  })
  return cached
}

export function parseSchema(json: string): JsonSchema {
  return JSON.parse(json) as JsonSchema
}

export function runGaps(
  api: WasmApi,
  save: Uint8Array,
  tokens: Uint8Array | undefined,
  defs: Uint8Array,
  goal: string,
): Promise<string> {
  if (!api.gaps) {
    return Promise.reject(new Error('Gaps analysis is unavailable in this wasm build.'))
  }
  return Promise.resolve(api.gaps(save, tokens, defs, '{}', goal))
}
