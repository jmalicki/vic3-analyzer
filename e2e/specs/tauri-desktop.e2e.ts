import { browser, $, expect } from '@wdio/globals'

describe('Tauri Desktop Companion', () => {
    before(function () {
        if (browser.capabilities.browserName !== 'tauri') {
            this.skip() // Only run these tests in Tauri
        }
    })

    it('should navigate to Dashboard and show auto-sync saves', async () => {
        // First check the dashboard default state
        const dashNoSave = await $('#dash-no-save')
        await expect(dashNoSave).toBeDisplayed()
        
        const savesTab = await $('#tab-saves')
        await savesTab.click()
        
        const saveList = await $('#saves-body')
        await expect(saveList).toBeExisting()
        
        const refreshBtn = await $('#refresh-saves')
        await refreshBtn.click()
        
        // We expect mock_save.v3 to appear
        const saveRow = await $('td*=mock_save.v3')
        await expect(saveRow).toBeExisting()
        
        // Try to load the save file
        // The user says "i can't click on a save file to load it!!"
        await saveRow.click()
        
        // Check the status text
        const status = await $('#saves-status')
        await expect(status).toHaveTextContaining('Loaded mock_save.v3')
        
        // Wait for dashboard to automatically show up since UI auto-navigates
        const dashActive = await $('#dash-active')
        await expect(dashActive).toBeDisplayed()
        
        const dashTitle = await $('#dash-title')
        await expect(dashTitle).toHaveTextContaining('Active Session:')
    })

    it('should run an Advanced Query', async () => {
        const queryTab = await $('#tab-query')
        await queryTab.click()
        
        const editor = await $('#sql-editor')
        await editor.setValue('SELECT goods_id, market_buy_orders, market_sell_orders FROM markets LIMIT 1;')
        
        const runBtn = await $('#run-sql')
        await runBtn.click()
        
        const results = await $('#results-body')
        await expect(results).toHaveTextContaining('mock_lumber')
    })
})
