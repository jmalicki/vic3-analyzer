import type { JsonSchema } from './types'

/**
 * Incremental defs build. Batching keeps the tab from holding a full install's
 * coat-of-arms art (400 MB+) in one array and copying it again into wasm.
 * Worker-backed builders return promises so the page can read the next batch
 * while this one is absorbed.
 */
export interface DefsBlobBuilder {
  addBatch(manifestJson: string, contents: Uint8Array): void | Promise<void>
  /** JSON array of lowercase gfx file names the added definitions reference. */
  neededGfxNames(): string | Promise<string>
  finish(): Uint8Array | Promise<Uint8Array>
  free?(): void
}

export interface WasmApi {
  classify_defs_path(path: string, isDirectory: boolean): 'read' | 'skip' | 'descend' | 'prune'
  DefsBlobBuilder: new () => DefsBlobBuilder
  build_defs_blob(
    manifestJson: string,
    contents: Uint8Array,
  ): Uint8Array | Promise<Uint8Array>
  defs_summary(defs: Uint8Array): string | Promise<string>
  /** JSON map of good id to a PNG data URL. */
  defs_icons(defs: Uint8Array): string | Promise<string>
  parse_save(save: Uint8Array, tokens?: Uint8Array): string | Promise<string>
  /** Build and retain the current save's world; returns summary and baseline prices. */
  load_analysis(
    save: Uint8Array,
    tokens: Uint8Array | undefined,
    defs: Uint8Array,
    solveOptsJson: string,
  ): string | Promise<string>
  clear_analysis(): void | Promise<void>
  loaded_prices(): string | Promise<string>
  loaded_military(): string | Promise<string>
  loaded_constructions(): string | Promise<string>
  /** Patch origin `.v3` bytes with a SavePatch JSON; returns new bytes. */
  export_save(
    originalBytes: Uint8Array,
    deltaJson: string,
  ): Uint8Array | Promise<Uint8Array>
  loaded_what_if(whatIfOptsJson: string): string | Promise<string>
  loaded_apply_delta(deltaJson: string): string | Promise<string>
  loaded_optimize_pms(axisJson: string): string | Promise<string>
  loaded_gaps(goal: string): string | Promise<string>
  loaded_plan(planOptsJson: string): string | Promise<string>
  loaded_alerts(): string | Promise<string>
  loaded_production_methods(): string | Promise<string>
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
  /** Glue-module import; tests stub this so init never hits the network. */
  importModule?: (url: string) => Promise<Record<string, unknown>>
}

let cached: Promise<WasmApi> | undefined

// Keep Vite from transforming a runtime public-asset import. wasm-pack's glue
// must remain a standalone module beside its `.wasm` file.
const nativeImport = Function('url', 'return import(url)') as (
  url: string,
) => Promise<Record<string, unknown>>

function wasmCacheBust(): string {
  return typeof __GIT_REVISION__ === 'string' && __GIT_REVISION__
    ? __GIT_REVISION__
    : 'dev'
}

/**
 * Root-absolute public/wasm URL. Must stay absolute: dynamic `import()` and
 * workers resolve relative URLs against the JS module path (`…/assets/…`), not
 * the app root — a `./wasm/…` glue URL then 404s as HTML and Chromium reports
 * `'text/html' is not a valid JavaScript MIME type`.
 *
 * `new URL(file, cacheBustedJsUrl)` also throws (query string), which is how
 * wasm-bindgen init used to break when given a relative sibling path.
 */
export function wasmPublicUrl(file: string): string {
  let base = import.meta.env.BASE_URL || '/'
  if (!base.startsWith('/')) {
    base = '/'
  }
  const prefix = base.endsWith('/') ? base : `${base}/`
  return `${prefix}wasm/${file}?v=${wasmCacheBust()}`
}

/** User-facing copy when the wasm glue or `.wasm` binary fails to load. */
export function formatAnalysisEngineLoadError(reason: unknown, sourceUrl?: string): string {
  const detail =
    reason instanceof Error
      ? reason.message
      : typeof reason === 'string'
        ? reason
        : reason != null
          ? String(reason)
          : ''
  if (detail.startsWith('Could not load the analysis engine')) {
    return detail
  }

  const lower = detail.toLowerCase()
  const where = sourceUrl ? ` Tried: ${sourceUrl}.` : ''
  const desktop =
    typeof globalThis !== 'undefined' &&
    typeof (globalThis as { window?: unknown }).window !== 'undefined' &&
    '__TAURI__' in (globalThis as { window: object }).window

  if (
    lower.includes('mime type') ||
    (lower.includes('text/html') && lower.includes('javascript'))
  ) {
    // Chromium says this when the glue URL 404s as HTML (wrong base / missing wasm).
    const fix = desktop
      ? 'Rebuild the desktop UI (`npm run build:desktop --prefix web`) and run the app again.'
      : 'The deployed UI is missing `/wasm/` assets or was built with the wrong Vite base.'
    return (
      `Could not load the analysis engine: the WebAssembly script URL returned a web page ` +
      `instead of JavaScript.${where} ${fix}`
    )
  }

  if (lower.includes('failed to construct') && lower.includes('url')) {
    return (
      `Could not load the analysis engine: could not build a URL for the WebAssembly module.${where}` +
      (detail ? ` (${detail})` : '')
    )
  }

  if (
    lower.includes('failed to fetch') ||
    lower.includes('networkerror') ||
    lower.includes('load failed') ||
    lower.includes('error loading dynamically imported module')
  ) {
    return (
      `Could not load the analysis engine: could not download the WebAssembly module.${where}` +
      (detail ? ` (${detail})` : ' Check that the wasm files are present next to the UI.')
    )
  }

  return detail
    ? `Could not load the analysis engine. ${detail}${where}`
    : `Could not load the analysis engine.${where || ''}`
}

export function loadWasm(options?: LoadWasmOptions): Promise<WasmApi> {
  const wasmPath = options?.moduleUrl ?? wasmPublicUrl('vic3_wasm.js')
  const modulePromise = options?.importModule
    ? options.importModule(wasmPath)
    : options?.moduleUrl
      ? import(/* @vite-ignore */ wasmPath)
      : nativeImport(wasmPath)
  cached ??= modulePromise
    .then(async (module) => {
      if (typeof module.default === 'function') {
        await module.default(
          options?.moduleOrPath !== undefined
            ? options.moduleOrPath
            : wasmPublicUrl('vic3_wasm_bg.wasm'),
        )
      }
      return module as unknown as WasmApi
    })
    .catch((error) => {
      cached = undefined
      if (
        error instanceof Error &&
        error.message.startsWith('Could not load the analysis engine')
      ) {
        throw error
      }
      throw new Error(formatAnalysisEngineLoadError(error, wasmPath), {
        cause: error,
      })
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
