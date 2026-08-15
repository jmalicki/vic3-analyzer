import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { zipSync } from 'fflate'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { DefsBuilder } from './DefsBuilder'
import { packDefsFiles } from './defsFiles'
import type { WasmApi } from './wasm'

function api(): WasmApi {
  return {
    build_defs_blob: vi.fn(() => new Uint8Array([1, 2, 3])),
  } as unknown as WasmApi
}

describe('DefsBuilder', () => {
  afterEach(cleanup)

  it('packs supported common files with byte offsets', () => {
    const packed = packDefsFiles([
      { path: 'game/common/goods/goods.txt', bytes: new Uint8Array([1, 2]) },
      { path: 'game/common/defines/defines.txt', bytes: new Uint8Array([3]) },
      { path: 'game/readme.txt', bytes: new Uint8Array([9]) },
    ])
    expect(Array.from(packed.contents)).toEqual([3, 1, 2])
    expect(JSON.parse(packed.manifestJson)).toEqual([
      { path: 'game/common/defines/defines.txt', offset: 0, length: 1 },
      { path: 'game/common/goods/goods.txt', offset: 1, length: 2 },
    ])
  })

  it('builds and selects a blob from a chosen game folder', async () => {
    const user = userEvent.setup()
    const wasm = api()
    const onBuilt = vi.fn()
    render(<DefsBuilder api={wasm} onBuilt={onBuilt} />)
    const file = new File(['grain = { cost = 20 }'], 'goods.txt')
    Object.defineProperty(file, 'webkitRelativePath', {
      value: 'Victoria 3/game/common/goods/goods.txt',
    })

    await user.upload(screen.getByLabelText('Victoria 3 definitions folder'), file)
    await waitFor(() => expect(wasm.build_defs_blob).toHaveBeenCalled())
    expect(onBuilt).toHaveBeenCalledWith(expect.objectContaining({ name: 'defs.postcard' }))
    expect(await screen.findByText(/Built defs.postcard from 1 definition files/)).toBeInTheDocument()
  })

  it('builds from a definitions zip', async () => {
    const user = userEvent.setup()
    const wasm = api()
    render(<DefsBuilder api={wasm} onBuilt={vi.fn()} />)
    const zip = zipSync({
      'game/common/goods/goods.txt': new TextEncoder().encode('grain = { cost = 20 }'),
    })

    await user.upload(
      screen.getByLabelText('Victoria 3 definitions zip'),
      new File([zip.slice().buffer as ArrayBuffer], 'definitions.zip', {
        type: 'application/zip',
      }),
    )
    await waitFor(() => expect(wasm.build_defs_blob).toHaveBeenCalled())
  })
})
