import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi, type Mock } from 'vitest'
import { DefsBuilder } from './DefsBuilder'
import {
  createJobQueue,
  DEFS_BATCH_BYTES,
  DEFS_BATCH_SIZE,
  DEFS_QUEUE_DEPTH,
  enumerateDroppedDefsFiles,
  packDefsFiles,
  streamDefsFiles,
  type DefsDropEntry,
  type DefsDropItem,
  type DefsFileSource,
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

function relativeFile(name: string): File {
  const file = new File(['grain = { cost = 20 }'], name)
  Object.defineProperty(file, 'webkitRelativePath', { value: `game/common/goods/${name}` })
  return file
}

/** A file whose read never settles, freezing progress at a known point. */
function hangingFile(name: string): File {
  const file = relativeFile(name)
  Object.defineProperty(file, 'arrayBuffer', { value: () => new Promise<ArrayBuffer>(() => {}) })
  return file
}

function hangingFileEntry(name: string): DefsDropEntry {
  return {
    isFile: true,
    isDirectory: false,
    name,
    file: (resolve: (file: File) => void) => resolve(hangingFile(name)),
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
    blob_version: 6,
    goods,
    labels: goods,
    icons: goods,
    production_methods: 412,
    pop_needs: 7,
    buy_packages: 5,
    price_range: 0.75,
  })
}

/** Records what the component streamed into wasm, batch by batch. */
type BuilderSpy = {
  batches: { manifestJson: string; contents: Uint8Array }[]
  finish: Mock<() => Uint8Array>
  addBatch?: Mock<(manifestJson: string, contents: Uint8Array) => void | Promise<void>>
  /** Art the fake wasm claims to reference; everything else must go unread. */
  neededGfxNames: string[]
}

function api(builder?: Partial<BuilderSpy>): WasmApi & { builder: BuilderSpy } {
  const spy: BuilderSpy = {
    batches: [],
    finish: builder?.finish ?? vi.fn(() => new Uint8Array([1, 2, 3])),
    addBatch: builder?.addBatch,
    neededGfxNames: builder?.neededGfxNames ?? [],
  }
  const wasm = {
    classify_defs_path: vi.fn(classify),
    DefsBlobBuilder: class {
      addBatch(manifestJson: string, contents: Uint8Array) {
        spy.batches.push({ manifestJson, contents })
        return spy.addBatch?.(manifestJson, contents)
      }
      neededGfxNames() {
        return JSON.stringify(spy.neededGfxNames)
      }
      finish() {
        return spy.finish()
      }
    },
    defs_summary: vi.fn(() => defsSummaryJson()),
  } as unknown as WasmApi
  return Object.assign(wasm, { builder: spy })
}

/** Drain the walk into one array, for tests about which files it picks. */
async function walkedPaths(
  items: DefsDropItem[],
  classify: DefsPathClassifier,
): Promise<string[]> {
  const sources = await enumerateDroppedDefsFiles(items, classify)
  return sources.map((source) => source.path)
}

/** Files the component actually handed to wasm, across every batch. */
function submittedPaths(spy: BuilderSpy): string[] {
  return spy.batches.flatMap(
    (batch) => (JSON.parse(batch.manifestJson) as { path: string }[]).map((entry) => entry.path),
  )
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

  it('caps streamed batches by source bytes as well as file count', async () => {
    const source = (path: string, size: number): DefsFileSource => ({
      path,
      size: async () => size,
      read: async () => new Uint8Array(size),
    })
    const sizes: number[][] = []

    await streamDefsFiles(
      [source('a', 3), source('b', 3), source('c', 1), source('large', 6)],
      async (batch) => {
        sizes.push(batch.map((file) => file.bytes.length))
      },
      { batchSize: 24, maxBatchBytes: 4 },
    )

    expect(sizes).toEqual([[3], [3, 1], [6]])
    expect(DEFS_BATCH_BYTES).toBe(4 * 1024 * 1024)
  })

  it('bounds the worker job queue so reads cannot run unbounded ahead', async () => {
    const started: number[] = []
    const releases: Array<() => void> = []
    const { enqueue, drain, size } = createJobQueue(2)
    const hang = (n: number) =>
      enqueue(
        () =>
          new Promise<void>((resolve) => {
            started.push(n)
            releases.push(resolve)
          }),
      )

    const first = hang(1)
    const second = hang(2)
    await Promise.resolve()
    expect(started).toEqual([1, 2])
    expect(size()).toBe(2)
    expect(DEFS_QUEUE_DEPTH).toBe(2)

    void hang(3)
    await Promise.resolve()
    expect(started).toEqual([1, 2])

    releases[0]()
    await first
    await waitFor(() => expect(started).toEqual([1, 2, 3]))
    expect(size()).toBe(2)
    releases[1]()
    releases[2]()
    await second
    await drain()
    expect(size()).toBe(0)
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
    await waitFor(() => expect(wasm.builder.finish).toHaveBeenCalled())
    expect(submittedPaths(wasm.builder)).toEqual(['Victoria 3/game/common/goods/goods.txt'])
    expect(onBuilt).toHaveBeenCalledWith(expect.objectContaining({ name: 'defs.postcard' }))
    expect(await screen.findByText(/Built defs.postcard format v6 from 1 definition files/)).toBeInTheDocument()
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

    expect(await walkedPaths([{ webkitGetAsEntry: () => common }], classify)).toEqual([
      'common/goods/00_goods.txt',
    ])
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
    const files = await walkedPaths([{ webkitGetAsEntry: () => game }], wasm.classify_defs_path)
    expect(files).toEqual([
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

    await waitFor(() => expect(wasm.builder.finish).toHaveBeenCalled())
    expect(onBuilt).toHaveBeenCalledWith(expect.objectContaining({ name: 'defs.postcard' }))
  })

  it('streams a large drop in batches instead of one buffer', async () => {
    const wasm = api()
    render(<DefsBuilder api={wasm} onBuilt={vi.fn()} />)
    // More files than one batch holds, so the walk must hand over several.
    const goods = Array.from({ length: DEFS_BATCH_SIZE * 2 + 3 }, (_, index) =>
      fileEntry(`${String(index).padStart(3, '0')}_goods.txt`, 'grain = { cost = 20 }'),
    )
    const common = dirEntry('common', [dirEntry('goods', goods)])

    fireEvent.drop(screen.getByLabelText('Drop the Victoria 3 game folder'), {
      dataTransfer: { files: [], items: [{ webkitGetAsEntry: () => common }] },
    })

    await waitFor(() => expect(wasm.builder.finish).toHaveBeenCalled())
    expect(wasm.builder.batches.length).toBe(3)
    expect(submittedPaths(wasm.builder)).toHaveLength(goods.length)
    for (const batch of wasm.builder.batches) {
      expect(JSON.parse(batch.manifestJson).length).toBeLessThanOrEqual(DEFS_BATCH_SIZE)
    }
  })

  it('counts a dropped tree before reading it so the bar is determinate', async () => {
    const wasm = api()
    render(<DefsBuilder api={wasm} onBuilt={vi.fn()} />)
    // The first read never settles, so the count on screen can only have come
    // from enumerating the tree up front.
    const goods = [
      hangingFileEntry('000_goods.txt'),
      ...Array.from({ length: 4 }, (_, index) =>
        fileEntry(`00${index + 1}_goods.txt`, 'grain = { cost = 20 }'),
      ),
    ]
    const common = dirEntry('common', [dirEntry('goods', goods)])

    fireEvent.drop(screen.getByLabelText('Drop the Victoria 3 game folder'), {
      dataTransfer: { files: [], items: [{ webkitGetAsEntry: () => common }] },
    })

    expect(await screen.findByText('Reading dropped files: 0 / 5')).toBeInTheDocument()
    expect(screen.getByRole('progressbar')).toHaveAttribute('max', '5')
  })

  it('keeps the chosen-folder bar determinate from the first file', async () => {
    const user = userEvent.setup()
    const wasm = api()
    render(<DefsBuilder api={wasm} onBuilt={vi.fn()} />)
    const files = [hangingFile('0_goods.txt'), relativeFile('1_goods.txt'), relativeFile('2_goods.txt')]

    await user.upload(screen.getByLabelText('Victoria 3 definitions folder'), files)

    expect(await screen.findByText('Reading definition files: 0 / 3')).toBeInTheDocument()
    expect(screen.getByRole('progressbar')).toHaveAttribute('max', '3')
  })

  it('never reads art the definitions do not reference', async () => {
    // A real install ships hundreds of unused emblems; reading them was most
    // of the wait, so the unused one here must stay untouched.
    const artClassify: DefsPathClassifier = (path, isDirectory) =>
      isDirectory ? 'descend' : path.endsWith('.txt') || path.endsWith('.dds') ? 'read' : 'skip'
    const wasm = api({ neededGfxNames: ['used.dds'] })
    wasm.classify_defs_path = vi.fn(artClassify)
    render(<DefsBuilder api={wasm} onBuilt={vi.fn()} />)

    const reads: string[] = []
    const trackedFile = (name: string): DefsDropEntry => ({
      isFile: true,
      isDirectory: false,
      name,
      file: (resolve: (file: File) => void) => {
        reads.push(name)
        resolve(new File(['x'], name))
      },
    })
    const game = dirEntry('game', [
      dirEntry('common', [dirEntry('goods', [fileEntry('00_goods.txt', 'grain = { cost = 20 }')])]),
      dirEntry('gfx', [
        dirEntry('coat_of_arms', [
          dirEntry('colored_emblems', [trackedFile('used.dds'), trackedFile('unused.dds')]),
        ]),
      ]),
    ])

    fireEvent.drop(screen.getByLabelText('Drop the Victoria 3 game folder'), {
      dataTransfer: { files: [], items: [{ webkitGetAsEntry: () => game }] },
    })

    await waitFor(() => expect(wasm.builder.finish).toHaveBeenCalled())
    expect(reads).toEqual(['used.dds'])
    expect(submittedPaths(wasm.builder)).toEqual([
      'game/common/goods/00_goods.txt',
      'game/gfx/coat_of_arms/colored_emblems/used.dds',
    ])
  })

  it('reads extra interface icons even when they are not in neededGfxNames', async () => {
    const artClassify: DefsPathClassifier = (path, isDirectory) =>
      isDirectory ? 'descend' : path.endsWith('.txt') || path.endsWith('.dds') ? 'read' : 'skip'
    const wasm = api({ neededGfxNames: ['grain'] })
    wasm.classify_defs_path = vi.fn(artClassify)
    render(<DefsBuilder api={wasm} onBuilt={vi.fn()} />)

    const reads: string[] = []
    const trackedFile = (name: string): DefsDropEntry => ({
      isFile: true,
      isDirectory: false,
      name,
      file: (resolve: (file: File) => void) => {
        reads.push(name)
        resolve(new File(['x'], name))
      },
    })
    const game = dirEntry('game', [
      dirEntry('common', [dirEntry('goods', [fileEntry('00_goods.txt', 'grain = { cost = 20 }')])]),
      dirEntry('gfx', [
        dirEntry('interface', [
          dirEntry('icons', [
            dirEntry('goods_icons', [trackedFile('grain.dds'), trackedFile('wood.dds')]),
            dirEntry('building_icons', [trackedFile('building_rye_farm.dds')]),
          ]),
        ]),
        dirEntry('coat_of_arms', [
          dirEntry('colored_emblems', [trackedFile('unused.dds')]),
        ]),
      ]),
    ])

    fireEvent.drop(screen.getByLabelText('Drop the Victoria 3 game folder'), {
      dataTransfer: { files: [], items: [{ webkitGetAsEntry: () => game }] },
    })

    await waitFor(() => expect(wasm.builder.finish).toHaveBeenCalled())
    expect(reads.sort()).toEqual(['building_rye_farm.dds', 'grain.dds'])
  })

  it('explains an empty drop instead of failing silently', async () => {
    render(<DefsBuilder api={api()} onBuilt={vi.fn()} />)

    fireEvent.drop(screen.getByLabelText('Drop the Victoria 3 game folder'), {
      dataTransfer: { files: [], items: [] },
    })

    expect(await screen.findByText(/no supported definitions/)).toBeInTheDocument()
  })

  it('keeps Choose game folder disabled until the analysis engine is ready', () => {
    render(<DefsBuilder onBuilt={vi.fn()} />)
    expect(screen.getByRole('button', { name: 'Choose game folder' })).toBeDisabled()
    expect(screen.getByRole('status')).toHaveTextContent('Waiting for the analysis engine')
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
    const wasm = api({
      finish: vi.fn(() => {
        throw new Error('bad goods file')
      }),
    })
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
    let release: (summary: string) => void = () => {}
    const wasm = api()
    wasm.defs_summary = vi.fn(() => new Promise<string>((resolve) => (release = resolve)))
    const onBusyChange = vi.fn()
    render(<DefsBuilder api={wasm} onBuilt={vi.fn()} onBusyChange={onBusyChange} />)
    const common = dirEntry('common', [
      dirEntry('goods', [fileEntry('00_goods.txt', 'grain = { cost = 20 }')]),
    ])

    fireEvent.drop(screen.getByLabelText('Drop the Victoria 3 game folder'), {
      dataTransfer: { files: [], items: [{ webkitGetAsEntry: () => common }] },
    })
    await waitFor(() => expect(onBusyChange).toHaveBeenCalledWith(true))
    await waitFor(() => expect(wasm.defs_summary).toHaveBeenCalled())

    release(defsSummaryJson())
    await waitFor(() => expect(onBusyChange).toHaveBeenLastCalledWith(false))
  })

  it('keeps a determinate file-count bar until queued absorbs finish', async () => {
    let release: (summary: string) => void = () => {}
    const wasm = api()
    wasm.defs_summary = vi.fn(() => new Promise<string>((resolve) => (release = resolve)))
    render(<DefsBuilder api={wasm} onBuilt={vi.fn()} />)
    const common = dirEntry('common', [
      dirEntry('goods', [fileEntry('00_goods.txt', 'grain = { cost = 20 }')]),
    ])

    fireEvent.drop(screen.getByLabelText('Drop the Victoria 3 game folder'), {
      dataTransfer: { files: [], items: [{ webkitGetAsEntry: () => common }] },
    })

    const bar = await screen.findByRole('progressbar', { name: 'Reading dropped files' })
    expect(bar).toHaveAttribute('max', '1')
    expect(screen.queryByText(/wasm/i)).not.toBeInTheDocument()
    await waitFor(() => expect(wasm.defs_summary).toHaveBeenCalled())
    expect(screen.getByText('Reading dropped files: 1 / 1')).toBeInTheDocument()

    release(defsSummaryJson())
    await waitFor(() => expect(screen.queryByRole('progressbar')).not.toBeInTheDocument())
  })

  it('does not count a file as done until the worker absorbs it', async () => {
    let release = () => {}
    const wasm = api({
      addBatch: vi.fn(() => new Promise<void>((resolve) => (release = resolve))),
    })
    render(<DefsBuilder api={wasm} onBuilt={vi.fn()} />)
    const common = dirEntry('common', [
      dirEntry('goods', [fileEntry('00_goods.txt', 'grain = { cost = 20 }')]),
    ])

    fireEvent.drop(screen.getByLabelText('Drop the Victoria 3 game folder'), {
      dataTransfer: { files: [], items: [{ webkitGetAsEntry: () => common }] },
    })

    expect(await screen.findByText('Reading dropped files: 0 / 1')).toBeInTheDocument()
    expect(screen.getByRole('progressbar')).toHaveAttribute('value', '0')
    expect(wasm.builder.finish).not.toHaveBeenCalled()

    release()
    await waitFor(() => expect(wasm.builder.finish).toHaveBeenCalled())
    expect(screen.queryByText(/wasm/i)).not.toBeInTheDocument()
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
