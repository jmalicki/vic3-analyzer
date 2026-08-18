/// <reference lib="webworker" />
import { loadWasm, type DefsBlobBuilder, type WasmApi } from './wasm'

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
let defsBuilder: DefsBlobBuilder | undefined
/** One wasm instance cannot overlap mutating calls; run the mailbox in order. */
let mailbox: Promise<void> = Promise.resolve()

self.addEventListener('message', (event: MessageEvent<WasmWorkerRequest>) => {
  mailbox = mailbox.then(
    () => respond(event.data),
    () => respond(event.data),
  )
})

async function respond({ id, method, args }: WasmWorkerRequest): Promise<void> {
  const post = (response: WasmWorkerResponse, transfer: Transferable[] = []) =>
    (self as unknown as Worker).postMessage(response, transfer)
  try {
    const api = await ready
    const value = await dispatch(api, method, args)
    // Blobs come back as fresh wasm copies, so hand over the buffer rather
    // than paying for a second one on the way out.
    const transfer = value instanceof Uint8Array ? [value.buffer as Transferable] : []
    post({ id, ok: true, value }, transfer)
  } catch (reason) {
    post({ id, ok: false, error: reason instanceof Error ? reason.message : String(reason) })
  }
}

async function dispatch(api: WasmApi, method: string, args: unknown[]): Promise<unknown> {
  switch (method) {
    case 'defs_builder_reset':
      defsBuilder?.free?.()
      defsBuilder = new api.DefsBlobBuilder()
      return undefined
    case 'defs_builder_add_batch':
      if (!defsBuilder) defsBuilder = new api.DefsBlobBuilder()
      await defsBuilder.addBatch(args[0] as string, args[1] as Uint8Array)
      return undefined
    case 'defs_builder_needed_gfx_names':
      if (!defsBuilder) throw new Error('definitions builder is not started')
      return defsBuilder.neededGfxNames()
    case 'defs_builder_finish': {
      if (!defsBuilder) throw new Error('definitions builder is not started')
      const bytes = await defsBuilder.finish()
      defsBuilder.free?.()
      defsBuilder = undefined
      return bytes
    }
    default: {
      const exports = api as unknown as Record<string, (...args: unknown[]) => unknown>
      return exports[method](...args)
    }
  }
}
