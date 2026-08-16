/// <reference lib="webworker" />
import { loadWasm } from './wasm'

/** One proxied call. `method` names an export on the wasm module. */
export type WasmWorkerRequest = {
  id: number
  method: string
  args: unknown[]
}

export type WasmWorkerResponse =
  | { id: number; ok: true; value: unknown }
  | { id: number; ok: false; error: string }

const ready = loadWasm()

self.addEventListener('message', (event: MessageEvent<WasmWorkerRequest>) => {
  void respond(event.data)
})

async function respond({ id, method, args }: WasmWorkerRequest): Promise<void> {
  const post = (response: WasmWorkerResponse, transfer: Transferable[] = []) =>
    (self as unknown as Worker).postMessage(response, transfer)
  try {
    const api = await ready
    const exports = api as unknown as Record<string, (...args: unknown[]) => unknown>
    const value = await exports[method](...args)
    // Blobs come back as fresh wasm copies, so hand over the buffer rather
    // than paying for a second one on the way out.
    const transfer = value instanceof Uint8Array ? [value.buffer as Transferable] : []
    post({ id, ok: true, value }, transfer)
  } catch (reason) {
    post({ id, ok: false, error: reason instanceof Error ? reason.message : String(reason) })
  }
}
