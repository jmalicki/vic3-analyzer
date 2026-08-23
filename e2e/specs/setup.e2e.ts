import { browser, $, expect } from '@wdio/globals'
import { join } from 'node:path'

describe('Setup & Configuration', () => {
    it('should load the web app', async () => {
        if (browser.capabilities.browserName !== 'tauri') {
            await browser.url('')
        }
        await expect($('body')).toBeExisting()
    })

    it('should configure game and save paths', async () => {
        if (browser.capabilities.browserName !== 'tauri') {
            // Web uses file inputs — desktop Settings is the intentional difference.
            return
        }

        // Settings is a workspace nav entry in the unified React UI (not the old #tab-settings).
        const settingsTab = await $('button*=Settings')
        await settingsTab.click()

        const gameInput = await $('#cfg-game')
        await gameInput.setValue(join(process.cwd(), '..', 'tests', 'fixtures', 'mock_game'))

        const saveInput = await $('#cfg-saves')
        await saveInput.setValue(join(process.cwd(), '..', 'tests', 'fixtures'))

        const saveBtn = await $('#save-settings')
        await saveBtn.click()

        const status = await $('#settings-status')
        await expect(status).toHaveText(expect.stringContaining('Saved'))
    })

    it('should not show the browser upload drop zone on desktop', async () => {
        if (browser.capabilities.browserName !== 'tauri') {
            return
        }
        await expect($('.inputs')).not.toBeExisting()
        await expect($('.desktop-catalog')).toBeExisting()
    })
})
