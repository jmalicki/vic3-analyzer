import { $, expect } from '@wdio/globals'
import { isTauriE2e, isWebE2e } from '../runtime.js'
import { SAVES } from '../fixtures.js'
import { bootWithSave } from '../session.js'

describe('00 Setup — paths + primary save load', () => {
  it('prepares multi-save fixtures and loads the shortage economy', async () => {
    await bootWithSave('shortage')

    if (isTauriE2e()) {
      await expect($('.inputs')).not.toBeExisting()
      await expect($('[aria-label="Loaded save"]')).toHaveText(
        expect.stringContaining(SAVES.shortage),
      )
    } else {
      await expect($('.inputs')).toBeExisting()
      await expect($('input[aria-label="Save file"]')).toBeExisting()
      await expect($('input[aria-label="Definitions file"]')).toBeExisting()
      await expect($('button=Download')).toBeEnabled()
    }

    if (isWebE2e()) {
      await expect($('nav[aria-label="Analysis tools"]')).toBeExisting()
    }
  })
})
