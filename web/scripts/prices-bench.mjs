// Times the wasm calls behind "Analyze prices", the way the page makes them.
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const webRoot = resolve(here, '..')
const savePath = process.argv[2]
const defsPath = process.argv[3]
// Binary saves need the token map, the same file the page asks for.
const tokensPath = process.argv[4] ?? process.env.VIC3_TOKENS
if (!savePath || !defsPath) {
  console.error('usage: node scripts/prices-bench.mjs <save.v3> <defs.postcard> [tokens.txt]')
  process.exit(1)
}

const wasmDir = resolve(webRoot, 'public/wasm')
const wasm = await import(pathToFileURL(resolve(wasmDir, 'vic3_wasm.js')).href)
await wasm.default(readFileSync(resolve(wasmDir, 'vic3_wasm_bg.wasm')))

const save = new Uint8Array(readFileSync(savePath))
const defs = new Uint8Array(readFileSync(defsPath))
const tokens = tokensPath ? new Uint8Array(readFileSync(tokensPath)) : undefined
const mb = (bytes) => (bytes / 1048576).toFixed(1)
console.log(`save ${mb(save.length)} MB, defs ${mb(defs.length)} MB${tokens ? ', with tokens' : ''}`)

const time = (name, run) => {
  const start = performance.now()
  const out = run()
  const ms = performance.now() - start
  console.log(`${name.padEnd(16)} ${ms.toFixed(0)}ms`)
  return { out, ms }
}

time('parse_save', () => wasm.parse_save(save, tokens))
const { out: pricesJson, ms } = time('prices', () => wasm.prices(save, tokens, defs, '{}'))
const result = JSON.parse(pricesJson)
console.log(`prices returned ${result.prices?.length ?? 0} goods, status ${result.status?.kind ?? '?'}`)
console.log(`total blocking time if run on the main thread: ${ms.toFixed(0)}ms`)
