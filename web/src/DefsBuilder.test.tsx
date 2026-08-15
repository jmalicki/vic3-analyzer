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
  afterEach(() => {
    cleanup()
    vi.restoreAllMocks()
  })

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

  it('shows a platform game/common path hint under the picker', () => {
    render(<DefsBuilder api={api()} onBuilt={vi.fn()} />)
    expect(screen.getByText('Usual Steam folder')).toBeInTheDocument()
    expect(document.querySelector('.path-hint-path')).toHaveTextContent(/game\/common|game\\common/)
    expect(screen.getByText(/Chrome cannot open that path for you|Cmd\+Shift\+G|Ctrl\+L/)).toBeInTheDocument()
  })

  it('shows a short explanation and deeper FieldHelp content', async () => {
    const user = userEvent.setup()
    render(<DefsBuilder api={api()} onBuilt={vi.fn()} />)
    expect(
      screen.getByText(/Prices need base costs and recipes from/),
    ).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Why definitions are needed' }))
    const help = await screen.findByRole('region', { name: 'Why definitions are needed' })
    expect(help).toHaveTextContent('Saves freeze the market situation')
    expect(help).toHaveTextContent('common/goods')
    expect(help).toHaveTextContent('never leave the browser')
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

  it('opens the folder input from the visible button', async () => {
    const user = userEvent.setup()
    render(<DefsBuilder api={api()} onBuilt={vi.fn()} />)
    const input = screen.getByLabelText('Victoria 3 definitions folder')
    const click = vi.spyOn(input, 'click')

    await user.click(screen.getByRole('button', { name: 'Choose game/common folder' }))
    expect(click).toHaveBeenCalled()
  })

  it('copies the platform path for pasting into the folder dialog', async () => {
    const user = userEvent.setup()
    const writeText = vi.fn(async () => {})
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })
    render(<DefsBuilder api={api()} onBuilt={vi.fn()} />)

    await user.click(screen.getByRole('button', { name: 'Copy path' }))
    expect(writeText).toHaveBeenCalledWith(expect.stringMatching(/game.common/))
    expect(await screen.findByText(/Path copied/)).toBeInTheDocument()
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
