/**
 * Which WDIO config launched this process. Set in wdio.*.conf.ts — do not
 * infer from `browser.capabilities.browserName` (WebKitGTK overwrites
 * `browserName: 'tauri'` and silently skips desktop suites).
 */
export type Vic3E2eRuntime = 'tauri' | 'web'

export function e2eRuntime(): Vic3E2eRuntime {
    const value = process.env.VIC3_E2E_RUNTIME
    if (value === 'tauri' || value === 'web') {
        return value
    }
    throw new Error(
        `VIC3_E2E_RUNTIME must be "tauri" or "web" (got ${JSON.stringify(value)}). ` +
            'Set it at the top of wdio.web.conf.ts / wdio.tauri.conf.ts.',
    )
}

export function isTauriE2e(): boolean {
    return e2eRuntime() === 'tauri'
}

export function isWebE2e(): boolean {
    return e2eRuntime() === 'web'
}
