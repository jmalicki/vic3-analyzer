import { browser, $, expect } from '@wdio/globals'

describe('Victoria 3 Analyzer App', () => {
    it('should load the app and display the main interface', async () => {
        // For Tauri, the app is already loaded via the service.
        // For Web, we navigate to the local dev server.
        if (browser.capabilities.browserName !== 'tauri') {
            await browser.url('')
        }

        // Use getTitle() — $('title').getText() is empty for <head> nodes in Chrome.
        await browser.waitUntil(
            async () => (await browser.getTitle()).includes('Victoria 3 Analyzer'),
            {
                timeout: 15_000,
                timeoutMsg: 'document title never contained Victoria 3 Analyzer',
            },
        )
        expect(await browser.getTitle()).toContain('Victoria 3 Analyzer')

        const body = await $('body')
        await expect(body).toBeExisting()
        await expect($('h1')).toHaveText(expect.stringContaining('Victoria 3 Analyzer'))
    })
})
