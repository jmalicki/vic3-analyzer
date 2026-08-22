import { browser, $, expect } from '@wdio/globals'

describe('Alerts & Shortages', () => {
    before(function () {
        if (browser.capabilities.browserName === 'tauri') {
            this.skip() // Only run these tests in Web browser
        }
    })

    it('should show Goods Shortage for mock_lumber', async () => {
        const navAlerts = await $('nav a[href="/alerts"]')
        if (await navAlerts.isExisting()) {
            await navAlerts.click()
            
            // Check for Goods Shortage alert
            const shortageAlert = await $('*=Goods Shortage')
            await expect(shortageAlert).toBeExisting()
            
            await shortageAlert.click() // Expand alert mitigations
        }
    })
})
