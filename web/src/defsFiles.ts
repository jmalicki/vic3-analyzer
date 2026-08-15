export type DefsSourceFile = {
  path: string
  bytes: Uint8Array
}

const DEF_DIRS = [
  'goods',
  'defines',
  'production_methods',
  'pop_needs',
  'buy_packages',
  'cultures',
]

export function usefulDefsPath(path: string): boolean {
  const normalized = path.replaceAll('\\', '/')
  return (
    normalized.endsWith('.txt') &&
    DEF_DIRS.some(
      (dir) =>
        normalized.includes(`/common/${dir}/`) || normalized.startsWith(`common/${dir}/`),
    )
  )
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

/**
 * Read dropped folders through the entries API.
 *
 * Dragging from the file manager is the one route Chromium does not restrict by
 * path, so it reaches a Steam install under `~/Library` or `Program Files` that
 * `showDirectoryPicker` refuses outright.
 */
export async function collectDroppedDefsFiles(
  items: Iterable<DefsDropItem>,
  onFile?: (read: number) => void,
): Promise<DefsSourceFile[]> {
  const out: DefsSourceFile[] = []
  for (const item of items) {
    const entry = item.webkitGetAsEntry?.()
    if (entry) await walkEntry(entry, entry.name, out, onFile)
  }
  return out
}

async function walkEntry(
  entry: DefsDropEntry,
  path: string,
  out: DefsSourceFile[],
  onFile?: (read: number) => void,
): Promise<void> {
  if (entry.isFile && entry.file) {
    if (!usefulDefsPath(path)) return
    const read = entry.file.bind(entry)
    const file = await new Promise<File>((resolve, reject) => read(resolve, reject))
    out.push({ path, bytes: new Uint8Array(await file.arrayBuffer()) })
    onFile?.(out.length)
    return
  }
  if (!entry.isDirectory || !entry.createReader) return
  const reader = entry.createReader()
  // readEntries yields a page at a time and signals completion with an empty batch.
  for (;;) {
    const batch = await new Promise<DefsDropEntry[]>((resolve, reject) =>
      reader.readEntries(resolve, reject),
    )
    if (batch.length === 0) return
    for (const child of batch) {
      await walkEntry(child, `${path}/${child.name}`, out, onFile)
    }
  }
}

export function packDefsFiles(files: DefsSourceFile[]): {
  manifestJson: string
  contents: Uint8Array
} {
  const selected = files
    .filter((file) => usefulDefsPath(file.path))
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
