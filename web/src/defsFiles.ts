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
  return found.map(({ entry, path }) => ({
    path,
    read: async () => new Uint8Array(await (await readEntryFile(entry)).arrayBuffer()),
  }))
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
      read: async () => new Uint8Array(await file.arrayBuffer()),
    }))
}

/**
 * Read sources in batches, handing each to `sink` before reading the next.
 *
 * A full install offers more than 400 MB of coat-of-arms art. Retaining all of
 * it — then copying it again into wasm — is what wedges the tab, so files are
 * released as soon as the sink has taken them. Within a batch the reads run
 * together, because each one is mostly waiting on the browser's file thread.
 */
export async function streamDefsFiles(
  sources: DefsFileSource[],
  sink: DefsBatchSink,
  options: { batchSize?: number; alreadyRead?: number } = {},
): Promise<number> {
  const { batchSize = DEFS_BATCH_SIZE } = options
  let read = options.alreadyRead ?? 0
  for (let start = 0; start < sources.length; start += batchSize) {
    const batch = await Promise.all(
      sources.slice(start, start + batchSize).map(async (source) => ({
        path: source.path,
        bytes: await source.read(),
      })),
    )
    read += batch.length
    await sink(batch, read)
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
 * Files per wasm handoff. Small enough that a 30 MB coat-of-arms batch is the
 * peak, large enough that per-call overhead stays invisible.
 */
export const DEFS_BATCH_SIZE = 24

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
