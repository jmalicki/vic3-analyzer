import { browser, $, expect } from '@wdio/globals'
import { join } from 'node:path'

describe('Setup & Configuration', () => {
    it('should load the web app', async () => {
        if (browser.capabilities.browserName !== 'tauri') {
            await browser.url('http://localhost:5173')
        }
        await expect($('body')).toBeExisting()
    })

    it('should configure game and save paths', async () => {
        if (browser.capabilities.browserName !== 'tauri') {
            // Web uses file inputs or mock loading
            // We can interact with the settings pane if one exists, 
            // but currently settings are for desktop. 
            // In web, users drag-and-drop or select dirs via file picker.
            return
        }
        
        // Switch to settings pane in Desktop companion
        const settingsTab = await $('#tab-settings')
        await settingsTab.click()
        
        const gameInput = await $('#cfg-game')
        await gameInput.setValue(join(__dirname, '..', '..', 'tests', 'fixtures', 'mock_game'))
        
        const saveInput = await $('#cfg-saves')
        await saveInput.setValue(join(__dirname, '..', '..', 'tests', 'fixtures'))
        
        const saveBtn = await $('#save-settings')
        await saveBtn.click()
        
        const status = await $('#settings-status')
        await expect(status).toHaveTextContaining('Saved')
    })
})
