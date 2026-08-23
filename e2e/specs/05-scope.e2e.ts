import { browser, $, $$, expect } from '@wdio/globals'
import { isTauriE2e } from '../runtime.js'
import { bootWithSave, openWorkspaceTab } from '../session.js'

/**
 * Player-scoped alerts + Domestic / Our market geography (PR #76 plan).
 * Uses mock_two_countries: MOCK/Home (player) vs RIVAL/Rivalia (foreign market).
 */
describe('05 Scope — foreign alerts hidden; domestic vs market geography', () => {
  before(async () => {
    await bootWithSave('twoCountries')
  })

  it('Alerts shows player Home pressure but not Rivalia alerts', async () => {
    await openWorkspaceTab('Alerts')
    await browser.waitUntil(
      async () =>
        (await $('*=Home needs').isExisting()) || (await $('*=mock_lumber shortage').isExisting()),
      { timeout: 60_000, timeoutMsg: 'Player-scoped alerts never appeared' },
    )
    await expect($('*=Rivalia')).not.toBeExisting()
    await expect($('*=Unmet pop needs in Rivalia')).not.toBeExisting()
    await expect($('*=Rivalia needs')).not.toBeExisting()
    const body = await $('section[aria-labelledby="alerts-heading"]')
    const text = await body.getText()
    if (!text.includes('Home') && !text.includes('mock_lumber')) {
      throw new Error(`expected player alerts, got: ${text.slice(0, 240)}`)
    }
  })

  it('States Our market / Domestic hide Rivalia; All shows it', async () => {
    await openWorkspaceTab('States')
    await expect($('button=Our market')).toBeExisting()
    await expect($('button=Domestic')).toBeExisting()
    await expect($('button=All')).toBeExisting()

    await $('button=Our market').click()
    await expect($('button=Our market')).toHaveAttribute('aria-pressed', 'true')
    await expect($('//a[contains(@class,"state-link") and contains(., "Home")]')).toBeExisting()
    await expect($('//a[contains(@class,"state-link") and contains(., "Rivalia")]')).not.toBeExisting()

    await $('button=Domestic').click()
    await expect($('button=Domestic')).toHaveAttribute('aria-pressed', 'true')
    await expect($('//a[contains(@class,"state-link") and contains(., "Home")]')).toBeExisting()
    await expect($('//a[contains(@class,"state-link") and contains(., "Rivalia")]')).not.toBeExisting()

    await $('button=All').click()
    await expect($('button=All')).toHaveAttribute('aria-pressed', 'true')
    await expect($('//a[contains(@class,"state-link") and contains(., "Home")]')).toBeExisting()
    await expect($('//a[contains(@class,"state-link") and contains(., "Rivalia")]')).toBeExisting()
  })

  it('Buildings Domestic keeps player camps; All includes foreign workshops', async () => {
    await openWorkspaceTab('Buildings')
    await $('button=Domestic').click()
    await expect($('button=Domestic')).toHaveAttribute('aria-pressed', 'true')
    await expect($('button[aria-label*="Mock Lumber Camp"]')).toBeExisting()

    await $('button=All').click()
    await expect($('button=All')).toHaveAttribute('aria-pressed', 'true')
    await expect($('button[aria-label*="Mock Tool Workshop"]')).toBeExisting()
  })

  it('Prices good drill-down scopes Rivalia out of Our market / Domestic', async () => {
    await openWorkspaceTab('Prices')
    const lumber = await $('a.good-link[href*="mock_lumber"]')
    if (await lumber.isExisting()) {
      await lumber.click()
    } else {
      const links = await $$('a.good-link')
      let clicked = false
      for (const link of links) {
        const text = await link.getText()
        if (text.toLowerCase().includes('lumber')) {
          await link.click()
          clicked = true
          break
        }
      }
      if (!clicked) {
        throw new Error('no lumber good-link to open')
      }
    }

    await browser.waitUntil(
      async () => (await $('button=Our market').isExisting()),
      { timeout: 30_000, timeoutMsg: 'Good detail never showed Our market scope control' },
    )

    await $('button=Our market').click()
    await expect($('//a[contains(@class,"state-link") and contains(., "Home")]')).toBeExisting()
    await expect($('//a[contains(@class,"state-link") and contains(., "Rivalia")]')).not.toBeExisting()
    await expect($('button[aria-label="Sort by State price"]')).toBeExisting()

    await $('button=Domestic').click()
    await expect($('//a[contains(@class,"state-link") and contains(., "Home")]')).toBeExisting()
    await expect($('//a[contains(@class,"state-link") and contains(., "Rivalia")]')).not.toBeExisting()

    await $('button=All').click()
    await expect($('//a[contains(@class,"state-link") and contains(., "Rivalia")]')).toBeExisting()
  })

  it('State Local Prices shows Market price and State price columns without Rivalia under Our market', async () => {
    // Land on Home state detail via hash (goods list has no state-links).
    await openWorkspaceTab('Prices')
    await browser.execute(() => {
      window.location.hash = '#/prices/state/1'
    })
    await browser.waitUntil(
      async () => (await $('button=Local Prices').isExisting()),
      { timeout: 15_000, timeoutMsg: 'Home state detail never mounted' },
    )

    const localTab = await $('button=Local Prices')
    await localTab.click()

    await expect($('th*=Market price')).toBeExisting()
    await expect($('th*=State price')).toBeExisting()
    await expect($('a.good-link[href*="mock_lumber"]')).toBeExisting()
    await expect($('*=Rivalia')).not.toBeExisting()
  })

  it('desktop SQL alerts() omits Rivalia while alerts(all) includes it', async function () {
    if (!isTauriE2e()) {
      this.skip()
    }
    await openWorkspaceTab('Query')
    const editor = await $('#sql-editor')
    await editor.setValue('SELECT title FROM alerts() ORDER BY title;')
    await $('#run-sql').click()
    await browser.waitUntil(
      async () =>
        (await $('#results-body').getText()).length > 0 || (await $('.alert.error').isExisting()),
      { timeout: 60_000, timeoutMsg: 'alerts() never returned' },
    )
    if (await $('.alert.error').isExisting()) {
      throw new Error(await $('.alert.error').getText())
    }
    const playerScoped = await $('#results-body').getText()
    if (playerScoped.includes('Rivalia')) {
      throw new Error(`alerts() leaked foreign rows: ${playerScoped}`)
    }

    await editor.setValue("SELECT title FROM alerts('all') ORDER BY title;")
    await $('#run-sql').click()
    await browser.waitUntil(
      async () => (await $('#results-body').getText()).includes('Rivalia'),
      { timeout: 60_000, timeoutMsg: "alerts('all') never showed Rivalia" },
    )
  })
})
