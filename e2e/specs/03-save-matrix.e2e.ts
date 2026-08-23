import { $, expect } from '@wdio/globals'
import { isTauriE2e } from '../runtime.js'
import { SAVE_MARKERS, SAVES } from '../fixtures.js'
import { bootWithSave, loadSave, openWorkspaceTab } from '../session.js'

/**
 * Different saves → different UI markers (PR #76 multi-save plan).
 * Fresh session: boot shortage, then switch to balanced in-process.
 */
describe('03 Save matrix — different results per save', () => {
  before(async () => {
    await bootWithSave('shortage')
  })

  it('shortage save shows lumber camp, tool workshop, and mock_lumber', async () => {
    await openWorkspaceTab('Prices')
    await expect($('a.good-link[href*="mock_lumber"]')).toBeExisting()

    await openWorkspaceTab('Buildings')
    await expect($('button[aria-label*="Mock Lumber Camp"]')).toBeExisting()
    await expect($('button[aria-label*="Mock Tool Workshop"]')).toBeExisting()
  })

  it('balanced save drops the tool workshop but keeps the lumber camp', async () => {
    await loadSave('balanced')

    await openWorkspaceTab('Buildings')
    await expect($('button[aria-label*="Mock Lumber Camp"]')).toBeExisting()
    await expect($('button[aria-label*="Mock Tool Workshop"]')).not.toBeExisting()

    await openWorkspaceTab('Prices')
    await expect($('a.good-link[href*="mock_lumber"]')).toBeExisting()

    if (isTauriE2e()) {
      await expect($('[aria-label="Loaded save"]')).toHaveText(
        expect.stringContaining(SAVES.balanced),
      )
    }
  })

  it('catalog lists all fixture saves on desktop', async function () {
    if (!isTauriE2e()) {
      this.skip()
    }
    await $('button*=Saves').click()
    await $('#refresh-saves').click()
    for (const name of Object.values(SAVES)) {
      await expect($(`td*=${name}`)).toBeExisting()
    }
    expect(SAVE_MARKERS.shortage.expectVisible.length).toBeGreaterThan(0)
  })
})
