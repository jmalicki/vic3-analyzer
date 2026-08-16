import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { DefsBuilder } from './DefsBuilder'
import {
  collectDroppedDefsFiles,
  packDefsFiles,
  type DefsDropEntry,
  type DefsPathClassifier,
} from './defsFiles'
import type { WasmApi } from './wasm'

const classify: DefsPathClassifier = (path, isDirectory) => {
  if (isDirectory) return path.endsWith('/gfx') || path === 'gfx' ? 'prune' : 'descend'
  return path.includes('/common/goods/') ||
    path.includes('/common/defines/') ||
    path.startsWith('common/goods/') ||
    path.startsWith('common/defines/') ||
    path.includes('/localization/english/goods_l_')
    ? 'read'
    : 'skip'
}

function fileEntry(name: string, contents: string): DefsDropEntry {
  return {
    isFile: true,
    isDirectory: false,
    name,
    file: (resolve: (file: File) => void) => resolve(new File([contents], name)),
  }
}

function dirEntry(name: string, children: DefsDropEntry[]): DefsDropEntry {
  let sent = false
  return {
    isFile: false,
    isDirectory: true,
    name,
    createReader: () => ({
      readEntries: (resolve: (entries: DefsDropEntry[]) => void) => {
        resolve(sent ? [] : children)
        sent = true
      },
    }),
  }
}

function defsSummaryJson(goods = 53): string {
  return JSON.stringify({
    blob_version: 3,
    goods,
    labels: goods,
    icons: goods,
    production_methods: 412,
    pop_needs: 7,
    buy_packages: 5,
    price_range: 0.75,
  })
}

function api(): WasmApi {
  return {
    classify_defs_path: vi.fn(classify),
    build_defs_blob: vi.fn(() => new Uint8Array([1, 2, 3])),
    defs_summary: vi.fn(() => defsSummaryJson()),
  } as unknown as WasmApi
}

describe('DefsBuilder', () => {
  afterEach(() => {
    cleanup()
    vi.restoreAllMocks()
  })

  it('packs supported common files with byte offsets', () => {
    const packed = packDefsFiles(
      [
        { path: 'game/common/goods/goods.txt', bytes: new Uint8Array([1, 2]) },
        { path: 'game/common/defines/defines.txt', bytes: new Uint8Array([3]) },
        { path: 'game/readme.txt', bytes: new Uint8Array([9]) },
      ],
      classify,
    )
    expect(Array.from(packed.contents)).toEqual([3, 1, 2])
    expect(JSON.parse(packed.manifestJson)).toEqual([
      { path: 'game/common/defines/defines.txt', offset: 0, length: 1 },
      { path: 'game/common/goods/goods.txt', offset: 1, length: 2 },
    ])
  })

  it('shows a platform game path hint under the picker', () => {
    render(<DefsBuilder api={api()} onBuilt={vi.fn()} />)
    expect(screen.getByText('Usual Steam folder')).toBeInTheDocument()
    expect(document.querySelector('.path-hint-path')).toHaveTextContent(/Victoria 3[\\/]game$/)
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
    expect(await screen.findByText(/Built defs.postcard format v3 from 1 definition files/)).toBeInTheDocument()
    expect(screen.getByRole('status')).toHaveTextContent(
      '53 goods, 53 localized names, 53 icons, 412 production methods',
    )
  })

  it('warns when the built blob has too few goods to be a real install', async () => {
    const user = userEvent.setup()
    const wasm = api()
    wasm.defs_summary = vi.fn(() => defsSummaryJson(2))
    render(<DefsBuilder api={wasm} onBuilt={vi.fn()} />)
    const file = new File(['grain = { cost = 20 }'], 'goods.txt')
    Object.defineProperty(file, 'webkitRelativePath', {
      value: 'common/goods/goods.txt',
    })

    await user.upload(screen.getByLabelText('Victoria 3 definitions folder'), file)

    expect(await screen.findByText(/common\/goods was probably missed/)).toBeInTheDocument()
  })

  it('walks a dropped folder tree through the entries API', async () => {
    const common = dirEntry('common', [
      dirEntry('goods', [fileEntry('00_goods.txt', 'grain = { cost = 20 }')]),
      dirEntry('art', [fileEntry('ignored.txt', 'x')]),
    ])

    const files = await collectDroppedDefsFiles([{ webkitGetAsEntry: () => common }], classify)
    expect(files.map((file) => file.path)).toEqual(['common/goods/00_goods.txt'])
  })

  it('accepts game root definitions and localization while pruning gfx', async () => {
    const game = dirEntry('game', [
      dirEntry('common', [
        dirEntry('goods', [fileEntry('00_goods.txt', 'grain = { cost = 20 }')]),
      ]),
      dirEntry('localization', [
        dirEntry('english', [fileEntry('goods_l_english.yml', 'grain:0 "Grain"')]),
      ]),
      dirEntry('gfx', [
        dirEntry('interface', [fileEntry('definitely-not-read.txt', 'huge')]),
      ]),
    ])

    const wasm = api()
    const files = await collectDroppedDefsFiles(
      [{ webkitGetAsEntry: () => game }],
      wasm.classify_defs_path,
    )
    expect(files.map((file) => file.path)).toEqual([
      'game/common/goods/00_goods.txt',
      'game/localization/english/goods_l_english.yml',
    ])
    expect(wasm.classify_defs_path).toHaveBeenCalledWith('game/gfx', true)
  })

  it('builds from a folder dragged onto the drop zone', async () => {
    const wasm = api()
    const onBuilt = vi.fn()
    render(<DefsBuilder api={wasm} onBuilt={onBuilt} />)
    const common = dirEntry('common', [
      dirEntry('goods', [fileEntry('00_goods.txt', 'grain = { cost = 20 }')]),
    ])

    fireEvent.drop(screen.getByLabelText('Drop the Victoria 3 game folder'), {
      dataTransfer: { files: [], items: [{ webkitGetAsEntry: () => common }] },
    })

    await waitFor(() => expect(wasm.build_defs_blob).toHaveBeenCalled())
    expect(onBuilt).toHaveBeenCalledWith(expect.objectContaining({ name: 'defs.postcard' }))
  })

  it('explains an empty drop instead of failing silently', async () => {
    render(<DefsBuilder api={api()} onBuilt={vi.fn()} />)

    fireEvent.drop(screen.getByLabelText('Drop the Victoria 3 game folder'), {
      dataTransfer: { files: [], items: [] },
    })

    expect(await screen.findByText(/no supported definitions/)).toBeInTheDocument()
  })

  it('opens the folder input from the visible button', async () => {
    const user = userEvent.setup()
    render(<DefsBuilder api={api()} onBuilt={vi.fn()} />)
    const input = screen.getByLabelText('Victoria 3 definitions folder')
    const click = vi.spyOn(input, 'click')

    await user.click(screen.getByRole('button', { name: 'Choose game folder' }))
    expect(click).toHaveBeenCalled()
  })

  it('stays open until the user acknowledges a finished build', async () => {
    const onDone = vi.fn()
    const onBuilt = vi.fn()
    render(<DefsBuilder api={api()} onBuilt={onBuilt} onDone={onDone} />)
    const common = dirEntry('common', [
      dirEntry('goods', [fileEntry('00_goods.txt', 'grain = { cost = 20 }')]),
    ])

    fireEvent.drop(screen.getByLabelText('Drop the Victoria 3 game folder'), {
      dataTransfer: { files: [], items: [{ webkitGetAsEntry: () => common }] },
    })

    expect(await screen.findByText(/Analysis tools are unlocked/)).toBeInTheDocument()
    expect(onBuilt).toHaveBeenCalled()
    expect(onDone).not.toHaveBeenCalled()

    await userEvent.setup().click(screen.getByRole('button', { name: 'OK' }))
    expect(onDone).toHaveBeenCalled()
  })

  it('shows an error when wasm rejects the definition files', async () => {
    const wasm = {
      ...api(),
      build_defs_blob: vi.fn(() => {
        throw new Error('bad goods file')
      }),
    } as unknown as WasmApi
    render(<DefsBuilder api={wasm} onBuilt={vi.fn()} onDone={vi.fn()} />)
    const common = dirEntry('common', [
      dirEntry('goods', [fileEntry('00_goods.txt', 'grain = {')]),
    ])

    fireEvent.drop(screen.getByLabelText('Drop the Victoria 3 game folder'), {
      dataTransfer: { files: [], items: [{ webkitGetAsEntry: () => common }] },
    })

    expect(await screen.findByRole('alert')).toHaveTextContent('bad goods file')
    expect(screen.queryByRole('progressbar')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'OK' })).not.toBeInTheDocument()
  })

  it('reports busy state so the dialog cannot be dismissed mid-build', async () => {
    let release: (bytes: Uint8Array) => void = () => {}
    const wasm = {
      ...api(),
      build_defs_blob: vi.fn(() => new Promise<Uint8Array>((resolve) => (release = resolve))),
    } as unknown as WasmApi
    const onBusyChange = vi.fn()
    render(<DefsBuilder api={wasm} onBuilt={vi.fn()} onBusyChange={onBusyChange} />)
    const common = dirEntry('common', [
      dirEntry('goods', [fileEntry('00_goods.txt', 'grain = { cost = 20 }')]),
    ])

    fireEvent.drop(screen.getByLabelText('Drop the Victoria 3 game folder'), {
      dataTransfer: { files: [], items: [{ webkitGetAsEntry: () => common }] },
    })
    await waitFor(() => expect(onBusyChange).toHaveBeenCalledWith(true))

    release(new Uint8Array([1, 2, 3]))
    await waitFor(() => expect(onBusyChange).toHaveBeenLastCalledWith(false))
  })

  it('reports progress while wasm parses the definitions', async () => {
    let release: (bytes: Uint8Array) => void = () => {}
    const wasm = {
      ...api(),
      build_defs_blob: vi.fn(
        () => new Promise<Uint8Array>((resolve) => (release = resolve)),
      ),
    } as unknown as WasmApi
    render(<DefsBuilder api={wasm} onBuilt={vi.fn()} />)
    const common = dirEntry('common', [
      dirEntry('goods', [fileEntry('00_goods.txt', 'grain = { cost = 20 }')]),
    ])

    fireEvent.drop(screen.getByLabelText('Drop the Victoria 3 game folder'), {
      dataTransfer: { files: [], items: [{ webkitGetAsEntry: () => common }] },
    })

    const bar = await screen.findByRole('progressbar', { name: 'Parsing definitions in wasm' })
    expect(bar).not.toHaveAttribute('value')

    release(new Uint8Array([1, 2, 3]))
    await waitFor(() => expect(screen.queryByRole('progressbar')).not.toBeInTheDocument())
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
    expect(writeText).toHaveBeenCalledWith(expect.stringMatching(/Victoria 3[\\/]game$/))
    expect(await screen.findByText(/Path copied/)).toBeInTheDocument()
  })

})
