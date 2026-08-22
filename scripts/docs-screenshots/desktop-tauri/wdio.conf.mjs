import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { dirname } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))

/** Filled by run.mjs before WDIO starts. */
const binary = process.env.VIC3_DOCS_TAURI_BIN
if (!binary) {
  throw new Error('VIC3_DOCS_TAURI_BIN is required (set by run.mjs)')
}

export const config = {
  runner: 'local',
  specs: [join(here, 'specs/capture.e2e.mjs')],
  maxInstances: 1,
  capabilities: [
    {
      browserName: 'tauri',
      'tauri:options': {
        application: binary,
      },
    },
  ],
  logLevel: 'info',
  framework: 'mocha',
  mochaOpts: {
    ui: 'bdd',
    timeout: 180_000,
  },
  services: [
    [
      '@wdio/tauri-service',
      {
        appBinaryPath: binary,
        driverProvider: 'embedded',
        // App already embeds the server via --features webdriver.
        waitForEmbeddedServerTimeout: 120_000,
      },
    ],
  ],
}
