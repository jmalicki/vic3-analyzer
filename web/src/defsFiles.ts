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

/**
 * Read dropped folders through the entries API, in small batches.
 *
 * Dragging from the file manager is the one route Chromium does not restrict by
 * path, so it reaches a Steam install under `~/Library` or `Program Files` that
 * `showDirectoryPicker` refuses outright.
 *
 * A full install offers more than 400 MB of coat-of-arms art. Retaining all of
 * it — then copying it again into wasm — is what wedges the tab, so files are
 * released as soon as the sink has taken them.
 */
export async function streamDroppedDefsFiles(
  items: Iterable<DefsDropItem>,
  classify: DefsPathClassifier,
  sink: DefsBatchSink,
  options: {
    batchSize?: number
    /** Reports the file count once the tree has been enumerated. */
    onTotal?: (total: number) => void
  } = {},
): Promise<number> {
  const { batchSize = DEFS_BATCH_SIZE, onTotal } = options
  // Enumerate before reading: directory listing is cheap next to pulling
  // bytes, and knowing the count up front makes the progress bar honest.
  const found: { entry: DefsDropEntry; path: string }[] = []
  for (const item of items) {
    const entry = item.webkitGetAsEntry?.()
    if (entry) await collectEntries(entry, entry.name, classify, found)
  }
  onTotal?.(found.length)

  let read = 0
  for (let start = 0; start < found.length; start += batchSize) {
    const batch: DefsSourceFile[] = []
    for (const { entry, path } of found.slice(start, start + batchSize)) {
      const file = await readEntryFile(entry)
      batch.push({ path, bytes: new Uint8Array(await file.arrayBuffer()) })
    }
    read += batch.length
    await sink(batch, read)
  }
  return read
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
