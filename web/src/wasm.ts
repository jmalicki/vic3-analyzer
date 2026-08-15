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
  gaps(
    save: Uint8Array,
    tokens: Uint8Array | undefined,
    defs: Uint8Array,
    solveOptsJson: string,
    goal: string,
  ): string | Promise<string>
  plan(
    save: Uint8Array,
    tokens: Uint8Array | undefined,
    defs: Uint8Array,
    solveOptsJson: string,
    planOptsJson: string,
  ): string | Promise<string>
  what_if_schema(): string
  prices_schema(): string
}

/** Options for loading wasm outside the default Pages/public URL (e.g. Vitest). */
export type LoadWasmOptions = {
  /** Module URL passed to dynamic `import()`. */
  moduleUrl?: string
  /** Bytes or URL passed to wasm-bindgen's default init (skips fetch of `.wasm`). */
  moduleOrPath?: BufferSource | string | URL
}

let cached: Promise<WasmApi> | undefined

function defaultModuleUrl(): string {
  const base = import.meta.env.BASE_URL || '/'
  const prefix = base.endsWith('/') ? base : `${base}/`
  return `${prefix}wasm/vic3_wasm.js`
}

export function loadWasm(options?: LoadWasmOptions): Promise<WasmApi> {
  const wasmPath = options?.moduleUrl ?? defaultModuleUrl()
  cached ??= import(/* @vite-ignore */ wasmPath).then(async (module) => {
    if (typeof module.default === 'function') {
      await module.default(
        options?.moduleOrPath !== undefined ? options.moduleOrPath : undefined,
      )
    }
    return module as unknown as WasmApi
  })
  return cached
}

/** Clear the cached module promise (tests only). */
export function resetWasmCache(): void {
  cached = undefined
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
  return Promise.resolve(api.gaps(save, tokens, defs, '{}', goal))
}
