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
  batchSize = DEFS_BATCH_SIZE,
): Promise<number> {
  let batch: DefsSourceFile[] = []
  let read = 0
  const flush = async () => {
    if (batch.length === 0) return
    read += batch.length
    const full = batch
    batch = []
    await sink(full, read)
  }
  for (const item of items) {
    const entry = item.webkitGetAsEntry?.()
    if (!entry) continue
    await walkEntryStreaming(entry, entry.name, classify, async (file) => {
      batch.push(file)
      if (batch.length >= batchSize) await flush()
    })
  }
  await flush()
  return read
}

/**
 * Files per wasm handoff. Small enough that a 30 MB coat-of-arms batch is the
 * peak, large enough that per-call overhead stays invisible.
 */
export const DEFS_BATCH_SIZE = 24

async function walkEntryStreaming(
  entry: DefsDropEntry,
  path: string,
  classify: DefsPathClassifier,
  emit: (file: DefsSourceFile) => Promise<void>,
): Promise<void> {
  if (entry.isFile && entry.file) {
    if (!usefulDefsPath(path, classify)) return
    const read = entry.file.bind(entry)
    const file = await new Promise<File>((resolve, reject) => read(resolve, reject))
    await emit({ path, bytes: new Uint8Array(await file.arrayBuffer()) })
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
      await walkEntryStreaming(child, `${path}/${child.name}`, classify, emit)
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
