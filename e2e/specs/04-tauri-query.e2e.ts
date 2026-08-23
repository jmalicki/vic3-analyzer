import { browser, $, expect } from '@wdio/globals'
import { isTauriE2e } from '../runtime.js'
import { loadSave, openWorkspaceTab } from '../session.js'

describe('04 Tauri Query', () => {
  before(function () {
    if (!isTauriE2e()) {
      this.skip()
    }
  })

  it('runs SQL against the loaded shortage session', async () => {
    await loadSave('shortage')
    await openWorkspaceTab('Query')

    const editor = await $('#sql-editor')
    await expect(editor).toBeExisting()
    await editor.setValue('SELECT * FROM goods LIMIT 10;')
    await $('#run-sql').click()

    const results = await $('#results-body')
    await browser.waitUntil(
      async () => {
        const text = await results.getText()
        const err = await $('.alert.error')
        return text.toLowerCase().includes('mock_lumber') || (await err.isExisting())
      },
      {
        timeout: 60_000,
        timeoutMsg: 'SQL never returned mock_lumber (or an error)',
      },
    )
    const err = await $('.alert.error')
    if (await err.isExisting()) {
      throw new Error(`SQL error: ${await err.getText()}`)
    }
    await expect(results).toHaveText(expect.stringContaining('mock_lumber'))
  })
})
