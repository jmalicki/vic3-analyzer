import { $, expect } from '@wdio/globals'
import { isWebE2e } from '../runtime.js'
import { bootWithSave, openWorkspaceTab } from '../session.js'

/** Deeper interactions on the shortage save (fresh session). */
describe('02 Content — interactions on shortage save', () => {
  before(async () => {
    await bootWithSave('shortage')
  })

  it('expands Mock Lumber Camp and shows per-building what-if controls', async () => {
    await openWorkspaceTab('Buildings')
    const expand = await $('button[aria-label*="Expand Mock Lumber Camp"]')
    await expect(expand).toBeExisting()
    await expand.click()
    await expect($('button=Run what-if')).toBeExisting()
    await expect($('button=Optimize production methods')).toBeExisting()
  })

  it('switches Build Queues to Private', async () => {
    await openWorkspaceTab('Buildings')
    await $('button=Queues').click()
    const privateTab = await $('button=Private')
    await expect(privateTab).toBeExisting()
    await privateTab.click()
    await expect(privateTab).toHaveAttribute('aria-selected', 'true')
  })

  it('sorts Prices and opens the mock_lumber row', async () => {
    await openWorkspaceTab('Prices')
    const priceHeader = await $('button[aria-label="Sort by Price"]')
    await expect(priceHeader).toBeExisting()
    await priceHeader.click()
    const lumber = await $('a.good-link[href*="mock_lumber"]')
    await expect(lumber).toBeExisting()
    await lumber.click()
  })

  it('expands the mock_lumber shortage alert', async () => {
    await openWorkspaceTab('Alerts')
    const shortageAlert = await $('*=mock_lumber shortage')
    await expect(shortageAlert).toBeExisting()
    await shortageAlert.click()
  })

  it('enables Download on web after load', async function () {
    if (!isWebE2e()) {
      this.skip()
    }
    const downloadBtn = await $('button=Download')
    await expect(downloadBtn).toBeExisting()
    await expect(downloadBtn).toBeEnabled()
  })
})
