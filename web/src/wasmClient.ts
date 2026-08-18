import { loadWasm, type WasmApi } from './wasm'
import type { WasmWorkerRequest, WasmWorkerResponse } from './wasmWorker'

/** The slice of `Worker` this client uses, so tests can supply a stand-in. */
export interface WasmWorkerPort {
  postMessage(message: WasmWorkerRequest): void
  addEventListener(
    type: 'message',
    listener: (event: MessageEvent<WasmWorkerResponse>) => void,
  ): void
}

/** Send calls to `port` and settle them against the replies. */
export function connectWasmWorker(port: WasmWorkerPort) {
  const pending = new Map<number, { resolve: (value: never) => void; reject: (error: Error) => void }>()
  let nextId = 1

  port.addEventListener('message', (event) => {
    const response = event.data
    const waiting = pending.get(response.id)
    if (!waiting) return
    pending.delete(response.id)
    if (response.ok) waiting.resolve(response.value as never)
    else waiting.reject(new Error(response.error))
  })

  return function call<T>(method: string, args: unknown[]): Promise<T> {
    const id = nextId++
    return new Promise<T>((resolve, reject) => {
      pending.set(id, { resolve: resolve as (value: never) => void, reject })
      port.postMessage({ id, method, args })
    })
  }
}

/**
 * A [`WasmApi`] whose analysis runs on a worker.
 *
 * Solving a market takes seconds of straight-line wasm, which freezes the tab
 * when it runs here: no repaint, no scrolling, not even the progress bar. Only
 * the allowlist and schema helpers — pure, microsecond calls — stay on this
 * thread, because the folder walk needs an answer without awaiting.
 */
export function workerWasmApi(local: WasmApi, port: WasmWorkerPort): WasmApi {
  const call = connectWasmWorker(port)
  return {
    classify_defs_path: (path, isDirectory) => local.classify_defs_path(path, isDirectory),
    DefsBlobBuilder: local.DefsBlobBuilder,
    what_if_schema: () => local.what_if_schema(),
    prices_schema: () => local.prices_schema(),
    build_defs_blob: (manifestJson, contents) =>
      call('build_defs_blob', [manifestJson, contents]),
    defs_summary: (defs) => call('defs_summary', [defs]),
    defs_icons: (defs) => call('defs_icons', [defs]),
    parse_save: (save, tokens) => call('parse_save', [save, tokens]),
    load_analysis: (save, tokens, defs, solveOptsJson) =>
      call('load_analysis', [save, tokens, defs, solveOptsJson]),
    clear_analysis: () => call('clear_analysis', []),
    loaded_prices: () => call('loaded_prices', []),
    loaded_what_if: (whatIfOptsJson) => call('loaded_what_if', [whatIfOptsJson]),
    loaded_gaps: (goal) => call('loaded_gaps', [goal]),
    loaded_plan: (planOptsJson) => call('loaded_plan', [planOptsJson]),
    loaded_alerts: () => call('loaded_alerts', []),
    // Keep the public byte-oriented API stable while production calls reuse
    // the worker-owned session loaded by App.
    prices: (_save, _tokens, _defs, _solveOptsJson) => call('loaded_prices', []),
    what_if: (_save, _tokens, _defs, _solveOptsJson, whatIfOptsJson) =>
      call('loaded_what_if', [whatIfOptsJson]),
    gaps: (_save, _tokens, _defs, _solveOptsJson, goal) => call('loaded_gaps', [goal]),
    plan: (_save, _tokens, _defs, _solveOptsJson, planOptsJson) =>
      call('loaded_plan', [planOptsJson]),
  }
}

/** Load wasm here for the cheap helpers and on a worker for everything else. */
export async function loadWasmApi(): Promise<WasmApi> {
  const worker = new Worker(new URL('./wasmWorker.ts', import.meta.url), { type: 'module' })
  return workerWasmApi(await loadWasm(), worker)
}
