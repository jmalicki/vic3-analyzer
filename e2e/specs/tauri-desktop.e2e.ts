import { browser, $, expect } from '@wdio/globals'

describe('Tauri Desktop Companion', () => {
    before(function () {
        if (browser.capabilities.browserName !== 'tauri') {
            this.skip() // Only run these tests in Tauri
        }
    })

    it('should navigate to Dashboard and show auto-sync saves', async () => {
        const savesTab = await $('#tab-saves')
        await savesTab.click()
        
        const saveList = await $('#saves-body')
        await expect(saveList).toBeExisting()
        
        const refreshBtn = await $('#refresh-saves')
        await refreshBtn.click()
        
        // We expect mock_save.v3 to appear
        await expect(saveList).toHaveTextContaining('mock_save.v3')
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
