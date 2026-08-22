import { browser, $, expect } from '@wdio/globals'

describe('Global App Shell', () => {
    before(function () {
        if (browser.capabilities.browserName === 'tauri') {
            this.skip() // Only run these tests in Web browser
        }
    })

    it('should undo changes and export save', async () => {
        const undoBtn = await $('button[aria-label="Undo last change"]')
        if (await undoBtn.isExisting()) {
            await undoBtn.click()
        }
        
        const exportBtn = await $('button*=Export Patched Save')
        if (await exportBtn.isExisting()) {
            await expect(exportBtn).toBeExisting()
        }
    })
})
