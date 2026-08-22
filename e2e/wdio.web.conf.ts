import { resolve } from 'node:path'

export const config = {
    runner: 'local',
    specs: [
        './specs/**/*.ts'
    ],
    maxInstances: 1,
    capabilities: [{
        browserName: 'chrome',
        'goog:chromeOptions': {
            args: ['--headless=new', '--disable-gpu', '--no-sandbox', '--disable-dev-shm-usage']
        }
    }],
    logLevel: 'info',
    bail: 0,
    baseUrl: 'http://localhost:5173/vic3-analyzer/',
    waitforTimeout: 10000,
    connectionRetryTimeout: 120000,
    connectionRetryCount: 3,
    services: [],
    framework: 'mocha',
    reporters: ['spec'],
    mochaOpts: {
        ui: 'bdd',
        timeout: 60000
    },
}
