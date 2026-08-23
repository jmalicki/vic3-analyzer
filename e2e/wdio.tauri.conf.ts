import { join } from 'node:path'
import { config as baseConfig } from './wdio.web.conf.js'

// Workspace cargo target; matches scripts/docs-screenshots/desktop-tauri/prepare.mjs
const binary = join(
    process.cwd(),
    '..',
    'target',
    'debug',
    process.platform === 'win32' ? 'vic3-analyzer.exe' : 'vic3-analyzer',
)

export const config = {
    ...baseConfig,
    runner: 'local',
    // Match the working desktop-tauri harness: capability key is `application`,
    // and the service needs `appBinaryPath` + embedded driver (requires
    // `cargo build -p vic3-analyzer --features webdriver`).
    services: [
        [
            '@wdio/tauri-service',
            {
                appBinaryPath: binary,
                driverProvider: 'embedded',
                waitForEmbeddedServerTimeout: 120_000,
                // Surface app stderr when the process dies before port 4445 is up
                // (common on headless Linux WebKit).
                captureBackendLogs: true,
            },
        ],
    ],
    capabilities: [{
        maxInstances: 1,
        browserName: 'tauri',
        'tauri:options': {
            application: binary,
        },
        // Prefer X11 under xvfb-run on Linux CI.
        'wdio:xvfbOptions': {
            xvfbArgs: ['-screen', '0', '1920x1080x24'],
        },
    }],
    baseUrl: 'tauri://localhost',
}
