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
    prices: (save, tokens, defs, solveOptsJson) =>
      call('prices', [save, tokens, defs, solveOptsJson]),
    what_if: (save, tokens, defs, solveOptsJson, whatIfOptsJson) =>
      call('what_if', [save, tokens, defs, solveOptsJson, whatIfOptsJson]),
    gaps: (save, tokens, defs, solveOptsJson, goal) =>
      call('gaps', [save, tokens, defs, solveOptsJson, goal]),
    plan: (save, tokens, defs, solveOptsJson, planOptsJson) =>
      call('plan', [save, tokens, defs, solveOptsJson, planOptsJson]),
  }
}

/** Load wasm here for the cheap helpers and on a worker for everything else. */
export async function loadWasmApi(): Promise<WasmApi> {
  const worker = new Worker(new URL('./wasmWorker.ts', import.meta.url), { type: 'module' })
  return workerWasmApi(await loadWasm(), worker)
}
