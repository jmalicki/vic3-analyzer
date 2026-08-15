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
