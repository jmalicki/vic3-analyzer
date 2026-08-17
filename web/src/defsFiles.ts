export type DefsSourceFile = {
  path: string
  bytes: Uint8Array
}

export type DefsPathClass = 'read' | 'skip' | 'descend' | 'prune'
export type DefsPathClassifier = (path: string, isDirectory: boolean) => DefsPathClass

export function usefulDefsPath(path: string, classify: DefsPathClassifier): boolean {
  return classify(path, false) === 'read'
}

export type DefsDropEntry = {
  isFile: boolean
  isDirectory: boolean
  name: string
  file?: (success: (file: File) => void, failure?: (error: unknown) => void) => void
  createReader?: () => {
    readEntries: (
      success: (entries: DefsDropEntry[]) => void,
      failure?: (error: unknown) => void,
    ) => void
  }
}

export type DefsDropItem = {
  webkitGetAsEntry?: () => DefsDropEntry | null
}

/** Receives one batch of files; resolves once the batch may be released. */
export type DefsBatchSink = (batch: DefsSourceFile[], read: number) => Promise<void>

/** A file located but not yet read. */
export type DefsFileSource = {
  path: string
  /** File size without loading its contents, used to bound each handoff. */
  size: () => Promise<number>
  read: () => Promise<Uint8Array>
}

/**
 * Locate the useful files in dropped folders, without reading any of them.
 *
 * Dragging from the file manager is the one route Chromium does not restrict by
 * path, so it reaches a Steam install under `~/Library` or `Program Files` that
 * `showDirectoryPicker` refuses outright.
 *
 * Listing directories is cheap next to pulling bytes, so doing it first buys
 * both an honest total and the chance to drop files nothing references.
 */
export async function enumerateDroppedDefsFiles(
  items: Iterable<DefsDropItem>,
  classify: DefsPathClassifier,
): Promise<DefsFileSource[]> {
  const found: { entry: DefsDropEntry; path: string }[] = []
  for (const item of items) {
    const entry = item.webkitGetAsEntry?.()
    if (entry) await collectEntries(entry, entry.name, classify, found)
  }
  return found.map(({ entry, path }) => {
    // Keep opening lazy so art rejected by neededGfxNames() is never touched.
    // Once selected, size() and read() share the same metadata-only File lookup.
    let file: Promise<File> | undefined
    const open = () => (file ??= readEntryFile(entry))
    return {
      path,
      size: async () => (await open()).size,
      read: async () => new Uint8Array(await (await open()).arrayBuffer()),
    }
  })
}

/** The useful files from a folder-picker selection. */
export function selectedDefsFiles(
  list: Iterable<File>,
  classify: DefsPathClassifier,
): DefsFileSource[] {
  return [...list]
    .filter((file) => usefulDefsPath(file.webkitRelativePath || file.name, classify))
    .map((file) => ({
      path: file.webkitRelativePath || file.name,
      size: async () => file.size,
      read: async () => new Uint8Array(await file.arrayBuffer()),
    }))
}

/**
 * Read sources in batches, handing each to `sink` before reading the next.
 *
 * A full install offers more than 400 MB of coat-of-arms art. Retaining all of
 * it — then copying it again into wasm — is what wedges the tab, so files are
 * released as soon as the sink has taken them. Batches are capped by both file
 * count and known source bytes, then read concurrently. A single source larger
 * than the byte cap is necessarily submitted alone.
 */
export async function streamDefsFiles(
  sources: DefsFileSource[],
  sink: DefsBatchSink,
  options: { batchSize?: number; maxBatchBytes?: number; alreadyRead?: number } = {},
): Promise<number> {
  const { batchSize = DEFS_BATCH_SIZE, maxBatchBytes = DEFS_BATCH_BYTES } = options
  let read = options.alreadyRead ?? 0
  for (let start = 0; start < sources.length; ) {
    let end = start
    let bytes = 0
    while (end < sources.length && end - start < batchSize) {
      const nextBytes = await sources[end].size()
      if (end > start && bytes + nextBytes > maxBatchBytes) break
      bytes += nextBytes
      end += 1
    }
    const batch = await Promise.all(
      sources.slice(start, end).map(async (source) => ({
        path: source.path,
        bytes: await source.read(),
      })),
    )
    read += batch.length
    await sink(batch, read)
    start = end
  }
  return read
}

/** True for the art files, which are the ones worth filtering by reference. */
export function isGfxDefsPath(path: string): boolean {
  // Segment-wise, so dropping the `gfx` folder itself is recognised too.
  return path
    .replace(/\\/g, '/')
    .toLowerCase()
    .split('/')
    .slice(0, -1)
    .includes('gfx')
}

/**
 * Match a path against the names wasm reported as referenced.
 *
 * Textures are named by file name, goods icons by stem, so try both.
 */
export function neededGfxFile(path: string, needed: ReadonlySet<string>): boolean {
  const name = path.replace(/\\/g, '/').split('/').pop()?.toLowerCase() ?? ''
  return needed.has(name) || needed.has(name.replace(/\.[^.]+$/, ''))
}

/**
 * Upper bounds per wasm handoff. The byte cap prevents a group of unusually
 * large textures from producing the old roughly 30 MB payload peak.
 */
export const DEFS_BATCH_SIZE = 24
export const DEFS_BATCH_BYTES = 4 * 1024 * 1024

function readEntryFile(entry: DefsDropEntry): Promise<File> {
  const read = entry.file!.bind(entry)
  return new Promise<File>((resolve, reject) => read(resolve, reject))
}

async function collectEntries(
  entry: DefsDropEntry,
  path: string,
  classify: DefsPathClassifier,
  found: { entry: DefsDropEntry; path: string }[],
): Promise<void> {
  if (entry.isFile && entry.file) {
    if (usefulDefsPath(path, classify)) found.push({ entry, path })
    return
  }
  if (!entry.isDirectory || !entry.createReader) return
  if (classify(path, true) === 'prune') return
  const reader = entry.createReader()
  // readEntries yields a page at a time and signals completion with an empty batch.
  for (;;) {
    const batch = await new Promise<DefsDropEntry[]>((resolve, reject) =>
      reader.readEntries(resolve, reject),
    )
    if (batch.length === 0) return
    for (const child of batch) {
      await collectEntries(child, `${path}/${child.name}`, classify, found)
    }
  }
}

export function packDefsFiles(files: DefsSourceFile[], classify: DefsPathClassifier): {
  manifestJson: string
  contents: Uint8Array
} {
  const selected = files
    .filter((file) => usefulDefsPath(file.path, classify))
    .sort((a, b) => a.path.localeCompare(b.path))
  const length = selected.reduce((total, file) => total + file.bytes.length, 0)
  const contents = new Uint8Array(length)
  let offset = 0
  const manifest = selected.map((file) => {
    contents.set(file.bytes, offset)
    const entry = { path: file.path, offset, length: file.bytes.length }
    offset += file.bytes.length
    return entry
  })
  return { manifestJson: JSON.stringify(manifest), contents }
}
