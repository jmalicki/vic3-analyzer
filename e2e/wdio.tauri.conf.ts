import { join } from 'node:path'
import { config as baseConfig } from './wdio.web.conf.js'

export const config = {
    ...baseConfig,
    runner: 'local',
    services: ['tauri'],
    capabilities: [{
        maxInstances: 1,
        'tauri:options': {
            applicationPath: join(__dirname, '..', 'crates', 'vic3-analyzer', 'target', 'debug', 'vic3-analyzer')
        }
    }],
    baseUrl: 'tauri://localhost'
}
