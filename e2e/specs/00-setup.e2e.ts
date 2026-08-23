import { browser, $, expect } from '@wdio/globals'
import { isTauriE2e, isWebE2e } from '../runtime.js'
import { prepareE2eSaves, SAVES } from '../fixtures.js'
import { configureTauriPaths, loadSave } from '../session.js'

describe('00 Setup — paths + primary save load', () => {
  it('opens the app', async () => {
    if (isWebE2e()) {
      await browser.url('')
    }
    await expect($('body')).toBeExisting()
    await browser.waitUntil(
      async () => (await browser.getTitle()).includes('Victoria 3 Analyzer'),
      { timeout: 15_000, timeoutMsg: 'title never contained Victoria 3 Analyzer' },
    )
  })

  it('prepares multi-save fixtures and loads the shortage economy', async () => {
    prepareE2eSaves()

    if (isTauriE2e()) {
      await configureTauriPaths()
      await expect($('.inputs')).not.toBeExisting()
      await expect($('[aria-label="Loaded save"]')).toBeExisting()
    } else {
      await expect($('.inputs')).toBeExisting()
      await expect($('input[aria-label="Save file"]')).toBeExisting()
      await expect($('input[aria-label="Definitions file"]')).toBeExisting()
    }

    await loadSave('shortage')

    if (isTauriE2e()) {
      await expect($('[aria-label="Loaded save"]')).toHaveText(
        expect.stringContaining(SAVES.shortage),
      )
    } else {
      await expect($('button=Download')).toBeEnabled()
    }
  })
})
