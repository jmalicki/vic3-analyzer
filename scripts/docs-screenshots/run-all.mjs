import { spawnSync } from 'node:child_process'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const HARD_TIMEOUT_MS = Number(process.env.DOCS_SCREENSHOTS_TIMEOUT_MS || 10 * 60 * 1000)

function run(script) {
  const result = spawnSync(process.execPath, [join(here, script)], {
    stdio: 'inherit',
    env: process.env,
    timeout: HARD_TIMEOUT_MS,
    killSignal: 'SIGKILL',
  })
  if (result.error) {
    console.error(result.error)
    process.exit(1)
  }
  if (result.status !== 0) process.exit(result.status ?? 1)
}

console.log(`docs screenshots (per-script timeout ${HARD_TIMEOUT_MS}ms)`)
run('capture-web.mjs')
run('capture-desktop-mock.mjs')
console.log('All docs screenshots generated.')
