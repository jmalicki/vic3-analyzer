/**
 * Build + run real macOS Tauri docs screenshots (native window chrome).
 */
import { spawnSync } from 'node:child_process'
import { platform } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { prepareAndBuild } from './prepare.mjs'

const here = dirname(fileURLToPath(import.meta.url))

if (platform() !== 'darwin') {
  console.error('docs:screenshots:desktop:tauri requires macOS (native window chrome).')
  process.exit(1)
}

// Clear a stale embedded WebDriver from a prior run.
spawnSync('pkill', ['-f', 'vic3-analyzer'], { stdio: 'ignore' })
spawnSync('bash', ['-lc', 'lsof -tiTCP:4445 | xargs kill -9 2>/dev/null || true'], {
  stdio: 'ignore',
})

const { env, binary } = prepareAndBuild()

const { existsSync } = await import('node:fs')
if (!existsSync(join(here, 'node_modules', '@wdio', 'tauri-service'))) {
  const pnpmInstall = spawnSync('pnpm', ['install', '--no-fund'], {
    cwd: here,
    stdio: 'inherit',
    env,
  })
  if (pnpmInstall.status !== 0) {
    process.exit(pnpmInstall.status ?? 1)
  }
}

const result = spawnSync(
  'npx',
  ['wdio', 'run', join(here, 'wdio.conf.mjs')],
  {
    cwd: here,
    stdio: 'inherit',
    env: {
      ...env,
      VIC3_DOCS_TAURI_BIN: binary,
    },
  },
)

process.exit(result.status ?? 1)
