/**
 * Seed XDG dirs for a disposable companion session (same idea as MCP smoke).
 * Writes a temporary WebDriver capability so default builds stay free of the plugin ACL.
 */
import { execFileSync } from 'node:child_process'
import {
  cpSync,
  mkdirSync,
  mkdtempSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { repoRoot, saveFixture } from '../lib/paths.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const webdriverCapability = join(
  repoRoot,
  'crates/vic3-analyzer/capabilities/webdriver.json',
)

/**
 * @returns {{ root: string, env: NodeJS.ProcessEnv, binary: string, cleanup: () => void }}
 */
export function prepareAndBuild() {
  const root = mkdtempSync(join(tmpdir(), 'vic3-docs-tauri-'))
  const xdg = join(root, 'xdg')
  const app = join(xdg, 'vic3-analyzer')
  const saves = join(root, 'saves')
  mkdirSync(app, { recursive: true })
  mkdirSync(saves, { recursive: true })
  mkdirSync(join(root, 'fake-home'), { recursive: true })

  cpSync(saveFixture, join(saves, 'autosave.v3'))

  const defsOut = join(app, 'defs.postcard')
  execFileSync(
    'cargo',
    ['run', '-q', '-p', 'vic3-defs', '--bin', 'emit_fixture_blob', '--', defsOut],
    { cwd: repoRoot, stdio: 'inherit' },
  )

  writeFileSync(
    join(app, 'config.toml'),
    [
      'auto_detect = false',
      `defs_blob = ${JSON.stringify(defsOut)}`,
      `save_dirs = [${JSON.stringify(saves)}]`,
      '',
    ].join('\n'),
  )

  writeFileSync(
    webdriverCapability,
    `${JSON.stringify(
      {
        identifier: 'webdriver',
        description: 'Companion UI + embedded WebDriver (docs screenshots only).',
        windows: ['main'],
        permissions: ['core:default', 'allow-companion', 'wdio-webdriver:default'],
      },
      null,
      2,
    )}\n`,
  )

  const env = {
    ...process.env,
    XDG_DATA_HOME: xdg,
    HOME: join(root, 'fake-home'),
    TAURI_CONFIG: JSON.stringify({
      app: { security: { capabilities: ['webdriver'] } },
    }),
  }

  try {
    execFileSync('cargo', ['build', '-p', 'vic3-analyzer', '--features', 'webdriver'], {
      cwd: repoRoot,
      stdio: 'inherit',
      env,
    })
  } finally {
    try {
      unlinkSync(webdriverCapability)
    } catch {
      /* ignore */
    }
  }

  return {
    root,
    env: {
      ...process.env,
      XDG_DATA_HOME: xdg,
      HOME: join(root, 'fake-home'),
    },
    binary: join(repoRoot, 'target/debug/vic3-analyzer'),
    cleanup: () => {},
  }
}

export { here, repoRoot }
