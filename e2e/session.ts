import { browser, $, expect } from '@wdio/globals'
import { join } from 'node:path'
import { isTauriE2e } from './runtime.js'
import { e2eSavesDir, mockGameDefsPostcard, mockGameDir, SAVES, type SaveKey } from './fixtures.js'

const ANALYSIS_TIMEOUT = 120_000

export async function openWorkspaceTab(label: string): Promise<void> {
  const tab = await $(`nav[aria-label="Analysis tools"] button*=${label}`)
  await expect(tab).toBeExisting()
  await tab.click()
}

/** Wait until analysis is priced (campaign HUD + goods prices, not empty state). */
export async function waitForAnalysisReady(): Promise<void> {
  await browser.waitUntil(
    async () => {
      const hud = await $('section[aria-label="Campaign summary"]')
      if (!(await hud.isExisting())) return false
      const empty = await $('p*=Prices appear after a save is priced')
      if (await empty.isExisting()) return false
      return (await $('#prices-tool-heading').isExisting())
    },
    {
      timeout: ANALYSIS_TIMEOUT,
      timeoutMsg: 'Analysis never became ready (HUD + Goods prices)',
    },
  )
}

export async function loadWebSave(saveKey: SaveKey): Promise<void> {
  const defsInput = await $('input[aria-label="Definitions file"]')
  await expect(defsInput).toBeExisting()
  await defsInput.setValue(mockGameDefsPostcard)
  await browser.waitUntil(
    async () => {
      const text = await $('small*=Using your file').getText().catch(() => '')
      return String(text).includes('mock_game.defs.postcard') || String(text).includes('Using your file')
    },
    { timeout: 30_000, timeoutMsg: 'Definitions file never applied' },
  )

  const saveInput = await $('input[aria-label="Save file"]')
  await expect(saveInput).toBeExisting()
  await saveInput.setValue(join(e2eSavesDir, SAVES[saveKey]))
  await waitForAnalysisReady()
}

export async function configureTauriPaths(): Promise<void> {
  const settingsTab = await $('button*=Settings')
  await settingsTab.click()

  await $('#cfg-game').setValue(mockGameDir)
  await $('#cfg-saves').setValue(e2eSavesDir)
  await $('#save-settings').click()
  await expect($('#settings-status')).toHaveText(expect.stringContaining('Saved'))
}

export async function loadTauriSave(saveKey: SaveKey): Promise<void> {
  const name = SAVES[saveKey]
  await $('button*=Saves').click()
  await $('#refresh-saves').click()

  await expect($(`td*=${name}`)).toBeExisting()
  await $(`tr*=${name}`).$('button*=Load').click()

  await browser.waitUntil(
    async () => (await $('#saves-status').getText()).includes(`Loaded ${name}`),
    {
      timeout: ANALYSIS_TIMEOUT,
      timeoutMsg: `Saves status never showed Loaded ${name}`,
    },
  )
  await waitForAnalysisReady()
}

export async function loadSave(saveKey: SaveKey): Promise<void> {
  if (isTauriE2e()) {
    await loadTauriSave(saveKey)
  } else {
    await loadWebSave(saveKey)
  }
}
