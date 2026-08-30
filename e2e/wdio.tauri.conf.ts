import { join } from 'node:path'
import { config as baseConfig } from './wdio.web.conf.js'

// Override after importing web conf (which sets VIC3_E2E_RUNTIME=web).
process.env.VIC3_E2E_RUNTIME = 'tauri'

// Workspace cargo target; CI sets CARGO_PROFILE=ci → target/ci/
function cargoTargetDir(): string {
    const profile = process.env.CARGO_PROFILE || 'dev'
    return profile === 'dev' ? 'debug' : profile
}

const binary = join(
    process.cwd(),
    '..',
    'target',
    cargoTargetDir(),
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
    // WebKit is slow; keep explicit waits in helpers rather than 10s default.
    waitforTimeout: 3000,
    mochaOpts: {
        ui: 'bdd',
        // Desktop Load + pricing + WebKit IPC can exceed 5 minutes on CI.
        timeout: 600_000,
    },
}
