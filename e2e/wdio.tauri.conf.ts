import { join } from 'node:path'
import { config as baseConfig } from './wdio.web.conf.js'

export const config = {
    ...baseConfig,
    runner: 'local',
    services: ['tauri'],
    capabilities: [{
        maxInstances: 1,
        browserName: 'tauri',
        'tauri:options': {
            applicationPath: join(process.cwd(), '..', 'target', 'debug', process.platform === 'win32' ? 'vic3-analyzer.exe' : 'vic3-analyzer')
        }
    }],
    baseUrl: 'tauri://localhost'
}
