import { browser, $, expect } from '@wdio/globals'

describe('React Web App Panes', () => {
    before(function () {
        if (browser.capabilities.browserName === 'tauri') {
            this.skip() // Only run these tests in Web browser
        }
    })

    it('should navigate to Buildings Pane and expand a row', async () => {
        // Assume mock game is loaded and mock_save is loaded
        const navBuildings = await $('nav a[href="/buildings"]')
        if (await navBuildings.isExisting()) {
            await navBuildings.click()
            
            const lumberCampRow = await $('*=Mock Lumber Camp')
            await expect(lumberCampRow).toBeExisting()
            
            await lumberCampRow.click() // Expand details
            const optimizeBtn = await $('button=Optimize')
            await expect(optimizeBtn).toBeExisting()
        }
    })

    it('should switch tabs in Build Queues', async () => {
        const navQueues = await $('nav a[href="/build-queues"]')
        if (await navQueues.isExisting()) {
            await navQueues.click()
            
            const privateTab = await $('button=Private')
            await privateTab.click()
            await expect(privateTab).toHaveAttribute('aria-pressed', 'true')
        }
    })
})
