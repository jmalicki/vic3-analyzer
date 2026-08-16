import { describe, expect, it, vi } from 'vitest'
import { workerWasmApi, type WasmWorkerPort } from './wasmClient'
import type { WasmApi } from './wasm'
import type { WasmWorkerRequest, WasmWorkerResponse } from './wasmWorker'

/** A worker stand-in that records calls and lets the test answer them. */
function fakePort() {
  const sent: WasmWorkerRequest[] = []
  let listener: ((event: MessageEvent<WasmWorkerResponse>) => void) | undefined
  const port: WasmWorkerPort = {
    postMessage: (message) => {
      sent.push(message)
    },
    addEventListener: (_type, handler) => {
      listener = handler
    },
  }
  const reply = (response: WasmWorkerResponse) => {
    listener?.({ data: response } as MessageEvent<WasmWorkerResponse>)
  }
  return { port, sent, reply }
}

function localApi(): WasmApi {
  return {
    classify_defs_path: vi.fn(() => 'read' as const),
    DefsBlobBuilder: class {
      addBatch() {}
      neededGfxNames() {
        return '[]'
      }
      finish() {
        return new Uint8Array()
      }
    },
    what_if_schema: vi.fn(() => '{"schema":"what-if"}'),
    prices_schema: vi.fn(() => '{"schema":"prices"}'),
    prices: vi.fn(() => {
      throw new Error('prices must not run on this thread')
    }),
    parse_save: vi.fn(() => {
      throw new Error('parse_save must not run on this thread')
    }),
  } as unknown as WasmApi
}

describe('workerWasmApi', () => {
  it('runs the analysis on the worker instead of this thread', async () => {
    const local = localApi()
    const { port, sent, reply } = fakePort()
    const api = workerWasmApi(local, port)

    const save = new Uint8Array([1, 2])
    const defs = new Uint8Array([3])
    const pending = api.prices(save, undefined, defs, '{}')

    expect(local.prices).not.toHaveBeenCalled()
    expect(sent).toEqual([
      { id: 1, method: 'prices', args: [save, undefined, defs, '{}'] },
    ])

    reply({ id: 1, ok: true, value: '{"prices":[]}' })
    expect(await pending).toBe('{"prices":[]}')
  })

  it('keeps the allowlist local, since the folder walk cannot await', () => {
    const local = localApi()
    const { port, sent } = fakePort()
    const api = workerWasmApi(local, port)

    expect(api.classify_defs_path('game/common/goods/00_goods.txt', false)).toBe('read')
    expect(local.classify_defs_path).toHaveBeenCalled()
    expect(sent).toEqual([])
  })

  it('settles each call against its own reply', async () => {
    const { port, sent, reply } = fakePort()
    const api = workerWasmApi(localApi(), port)

    const first = api.parse_save(new Uint8Array([1]))
    const second = api.defs_summary(new Uint8Array([2]))
    expect(sent.map((message) => message.method)).toEqual(['parse_save', 'defs_summary'])

    reply({ id: sent[1].id, ok: true, value: '{"goods":3}' })
    reply({ id: sent[0].id, ok: true, value: '{"tag":"GER"}' })
    expect(await first).toBe('{"tag":"GER"}')
    expect(await second).toBe('{"goods":3}')
  })

  it('surfaces a worker failure as a rejected call', async () => {
    const { port, sent, reply } = fakePort()
    const api = workerWasmApi(localApi(), port)

    const pending = api.prices(new Uint8Array(), undefined, new Uint8Array(), '{}')
    reply({ id: sent[0].id, ok: false, error: 'bad save file' })

    await expect(pending).rejects.toThrow('bad save file')
  })
})
