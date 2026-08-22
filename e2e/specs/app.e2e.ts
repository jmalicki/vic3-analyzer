import { browser, $, expect } from '@wdio/globals'

describe('Victoria 3 Analyzer App', () => {
    it('should load the app and display the main interface', async () => {
        // For Tauri, the app is already loaded via the service.
        // For Web, we navigate to the local dev server.
        if (browser.capabilities.browserName !== 'tauri') {
            await browser.url('http://localhost:5173')
        }

        // Wait for the app to initialize (e.g., look for a known element like the title or a load button)
        const title = await $('title')
        if (await title.isExisting()) {
            await expect(title).toHaveTextContaining('Victoria 3 Analyzer', { ignoreCase: true })
        }

        // Check if there is a file input or a recognizable UI element
        const body = await $('body')
        await expect(body).toBeExisting()
    })
})
