// Reproduces the browser defs build outside a tab: walk the install with the
// wasm allowlist, pack one contiguous buffer, then time build_defs_blob.
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, resolve, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const webRoot = resolve(here, '..')
const game = process.argv[2]
if (!game) {
  console.error('usage: node scripts/defs-bench.mjs "<path to Victoria 3/game>"')
  process.exit(1)
}

const wasmDir = resolve(webRoot, 'public/wasm')
const wasmModule = await import(pathToFileURL(resolve(wasmDir, 'vic3_wasm.js')).href)
await wasmModule.default(readFileSync(resolve(wasmDir, 'vic3_wasm_bg.wasm')))

const classify = (path, isDirectory) => wasmModule.classify_defs_path(path, isDirectory)

const files = []
function walk(absolute, relative) {
  if (classify(relative, true) === 'prune') return
  for (const name of readdirSync(absolute)) {
    const childAbsolute = join(absolute, name)
    const childRelative = `${relative}/${name}`
    const stats = statSync(childAbsolute)
    if (stats.isDirectory()) {
      walk(childAbsolute, childRelative)
    } else if (classify(childRelative, false) === 'read') {
      files.push({ path: childRelative, size: stats.size, absolute: childAbsolute })
    }
  }
}

const walkStart = performance.now()
walk(game, 'game')
const walkMs = performance.now() - walkStart

const totalBytes = files.reduce((sum, file) => sum + file.size, 0)
const byKind = new Map()
for (const file of files) {
  const kind = file.path.includes('/gfx/coat_of_arms/')
    ? 'coa textures'
    : file.path.includes('/goods_icons/')
      ? 'goods icons'
      : file.path.includes('/localization/')
        ? 'localization'
        : 'common txt'
  const entry = byKind.get(kind) ?? { count: 0, bytes: 0 }
  entry.count += 1
  entry.bytes += file.size
  byKind.set(kind, entry)
}

const mb = (bytes) => (bytes / 1048576).toFixed(1)
console.log(`walk: ${files.length} files, ${mb(totalBytes)} MB in ${walkMs.toFixed(0)}ms`)
for (const [kind, entry] of [...byKind].sort((a, b) => b[1].bytes - a[1].bytes)) {
  console.log(`  ${kind.padEnd(14)} ${String(entry.count).padStart(5)} files  ${mb(entry.bytes).padStart(7)} MB`)
}

files.sort((a, b) => a.path.localeCompare(b.path))

const batchSize = Number(process.env.BATCH ?? 24)
const streamStart = performance.now()
const builder = new wasmModule.DefsBlobBuilder()
let peakBatchBytes = 0
let longestBlockMs = 0

// Text first, then only the art the definitions actually name.
const isGfx = (file) => file.path.includes('/gfx/')
const text = files.filter((file) => !isGfx(file))
const submit = (batch) => {
  const bytes = batch.reduce((sum, file) => sum + file.size, 0)
  peakBatchBytes = Math.max(peakBatchBytes, bytes)
  const contents = new Uint8Array(bytes)
  let offset = 0
  const manifest = batch.map((file) => {
    contents.set(new Uint8Array(readFileSync(file.absolute)), offset)
    const entry = { path: file.path, offset, length: file.size }
    offset += file.size
    return entry
  })
  const blockStart = performance.now()
  builder.addBatch(JSON.stringify(manifest), contents)
  longestBlockMs = Math.max(longestBlockMs, performance.now() - blockStart)
}
for (let start = 0; start < text.length; start += batchSize) {
  submit(text.slice(start, start + batchSize))
}
const needed = new Set(JSON.parse(builder.neededGfxNames()))
const wanted = files.filter((file) => {
  if (!isGfx(file)) return false
  const parts = file.path.replace(/\\/g, '/').toLowerCase().split('/')
  const icons = parts.indexOf('icons')
  const extraIcon = icons >= 0 && parts[icons + 1] && parts[icons + 1] !== 'goods_icons'
  if (extraIcon) return true
  const name = file.path.split('/').pop().toLowerCase()
  return needed.has(name) || needed.has(name.replace(/\.[^.]+$/, ''))
})
const skipped = files.length - text.length - wanted.length
const skippedBytes = files
  .filter((file) => isGfx(file) && !wanted.includes(file))
  .reduce((sum, file) => sum + file.size, 0)
console.log(
  `skipped ${skipped} unreferenced gfx files (${mb(skippedBytes)} MB never read)`,
)
for (let start = 0; start < wanted.length; start += batchSize) {
  submit(wanted.slice(start, start + batchSize))
}

const finishStart = performance.now()
const blob = builder.finish()
const finishMs = performance.now() - finishStart
longestBlockMs = Math.max(longestBlockMs, finishMs)

console.log(`stream (batch=${batchSize}): ${(performance.now() - streamStart).toFixed(0)}ms total`)
console.log(`  finish():           ${finishMs.toFixed(0)}ms`)
console.log(`  longest main-thread block: ${longestBlockMs.toFixed(0)}ms`)
console.log(`  peak batch payload: ${mb(peakBatchBytes)} MB (was ${mb(totalBytes)} MB in one array)`)
console.log(`blob: ${mb(blob.length)} MB`)

if (process.env.COMPARE) {
  const contents = new Uint8Array(totalBytes)
  let offset = 0
  const manifest = files.map((file) => {
    contents.set(new Uint8Array(readFileSync(file.absolute)), offset)
    const entry = { path: file.path, offset, length: file.size }
    offset += file.size
    return entry
  })
  const oneShot = wasmModule.build_defs_blob(JSON.stringify(manifest), contents)
  const same = oneShot.length === blob.length && oneShot.every((byte, i) => byte === blob[i])
  console.log(`identical to one-shot build_defs_blob: ${same}`)
}
console.log(`summary: ${wasmModule.defs_summary(blob)}`)
console.log(`peak rss: ${mb(process.memoryUsage().rss)} MB`)
