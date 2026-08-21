import { spawnSync } from 'node:child_process'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))

function run(script) {
  const result = spawnSync(process.execPath, [join(here, script)], {
    stdio: 'inherit',
    env: process.env,
  })
  if (result.status !== 0) process.exit(result.status ?? 1)
}

run('capture-web.mjs')
run('capture-desktop-mock.mjs')
console.log('All docs screenshots generated.')
