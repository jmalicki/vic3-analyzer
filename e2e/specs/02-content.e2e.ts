import { $, expect } from '@wdio/globals'
import { isWebE2e } from '../runtime.js'
import { loadSave, openWorkspaceTab } from '../session.js'

/** Deeper interactions on top of 01’s content checks (after possible matrix switches). */
describe('02 Content — interactions on shortage save', () => {
  before(async () => {
    await loadSave('shortage')
  })

  it('expands Mock Lumber Camp and shows Optimize', async () => {
    await openWorkspaceTab('Buildings')
    const lumberCampRow = await $('*=Mock Lumber Camp')
    await expect(lumberCampRow).toBeExisting()
    await lumberCampRow.click()
    await expect($('button=Optimize')).toBeExisting()
  })

  it('switches Build Queues to Private', async () => {
    await openWorkspaceTab('Buildings')
    await $('button=Queues').click()
    const privateTab = await $('button=Private')
    await expect(privateTab).toBeExisting()
    await privateTab.click()
    await expect(privateTab).toHaveAttribute('aria-pressed', 'true')
  })

  it('sorts Prices and opens the mock_lumber row', async () => {
    await openWorkspaceTab('Prices')
    const priceHeader = await $('button*=Sort by Price')
    await expect(priceHeader).toBeExisting()
    await priceHeader.click()
    const lumberRow = await $('*=mock_lumber')
    await expect(lumberRow).toBeExisting()
    await lumberRow.click()
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
