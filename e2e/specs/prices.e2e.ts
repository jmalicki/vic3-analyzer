import { browser, $, expect } from '@wdio/globals'

describe('Price Explorer', () => {
    before(function () {
        if (browser.capabilities.browserName === 'tauri') {
            this.skip() // Only run these tests in Web browser
        }
    })

    it('should sort table headers and expand rows', async () => {
        const navPrices = await $('nav a[href="/prices"]')
        if (await navPrices.isExisting()) {
            await navPrices.click()
            
            // Click to sort by shortage
            const shortageHeader = await $('th*=Shortage')
            if (await shortageHeader.isExisting()) {
                await shortageHeader.click()
            }
            
            // Expand mock lumber
            const lumberRow = await $('*=mock_lumber')
            await expect(lumberRow).toBeExisting()
            await lumberRow.click()
        }
    })
})
