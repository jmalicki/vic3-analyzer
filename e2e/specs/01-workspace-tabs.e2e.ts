import { browser, $, expect } from '@wdio/globals'
import { isTauriE2e, isWebE2e } from '../runtime.js'
import { bootWithSave, openWorkspaceTab } from '../session.js'

/**
 * Fresh session per spec file: boot shortage, then verify each workspace pane
 * shows shortage-economy content — not just that the tab heading mounts.
 *
 * UI labels use displayId (mock_lumber → "Mock Lumber"); prefer href / aria-label.
 */
describe('01 Workspace tabs — content', () => {
  before(async () => {
    await bootWithSave('shortage')
  })

  it('Prices lists mock goods from the shortage solve', async () => {
    await openWorkspaceTab('Prices')
    await expect($('#prices-tool-heading')).toHaveText('Goods prices')
    await expect($('p*=Prices appear after a save is priced')).not.toBeExisting()
    await expect($('a.good-link[href*="mock_lumber"]')).toBeExisting()
    await expect($('a.good-link[href*="mock_tools"]')).toBeExisting()
    await expect($('a.good-link[href*="mock_iron"]')).toBeExisting()
    await expect($('button[aria-label="Sort by Price"]')).toBeExisting()
  })

  it('States lists STATE_MOCK / Mock with a state table', async () => {
    await openWorkspaceTab('States')
    await expect($('#states-heading')).toHaveText('States')
    await expect($('*=1 states')).toBeExisting()
    await expect($('a.state-link[href*="/states/1"]')).toBeExisting()
    await expect($('//a[contains(@class,"state-link") and contains(., "Mock")]')).toBeExisting()
    await expect($('button[aria-label="Sort by Name"]')).toBeExisting()
    await expect($('p*=Load a save to see states')).not.toBeExisting()
  })

  it('Pops lists mock_laborers from the shortage save', async () => {
    await openWorkspaceTab('Pops')
    await expect($('#pops-heading')).toHaveText('Pops')
    await expect($('*=Mock Laborers')).toBeExisting()
    await expect($('p*=Load a save to see pops')).not.toBeExisting()
    await expect($('p*=No pops in this scope')).not.toBeExisting()
  })

  it('Alerts includes the mock_lumber shortage', async () => {
    await openWorkspaceTab('Alerts')
    await expect($('#alerts-heading')).toHaveText('Alerts')
    await browser.waitUntil(
      async () => (await $('*=mock_lumber shortage').isExisting()),
      {
        timeout: 60_000,
        timeoutMsg: 'Alerts never showed mock_lumber shortage',
      },
    )
    await expect($('*=Farmers')).toBeExisting()
  })

  it('Military renders branch tabs and empty armies for this save', async () => {
    await openWorkspaceTab('Military')
    await expect($('#military-heading')).toHaveText('Military')
    await expect($('button=Army')).toBeExisting()
    await expect($('button=Navy')).toBeExisting()
    await expect($('button=Mobilization')).toBeExisting()
    await browser.waitUntil(
      async () =>
        (await $('*=No armies recorded in this save').isExisting())
        || (await $('*=Military details appear after a save is priced').isExisting()),
      {
        timeout: 60_000,
        timeoutMsg: 'Military pane never resolved armies empty state',
      },
    )
    if (await $('*=Military details appear after a save is priced').isExisting()) {
      await browser.waitUntil(
        async () => (await $('*=No armies recorded in this save').isExisting()),
        { timeout: 60_000, timeoutMsg: 'Military snapshot never arrived' },
      )
    }
    await expect($('*=No armies recorded in this save')).toBeExisting()
    await $('button=Navy').click()
    await expect($('*=No navies recorded in this save')).toBeExisting()
  })

  it('Buildings lists lumber camp and tool workshop', async () => {
    await openWorkspaceTab('Buildings')
    await expect($('#buildings-heading')).toHaveText('Buildings')
    await expect($('button[aria-label*="Mock Lumber Camp"]')).toBeExisting()
    await expect($('button[aria-label*="Mock Tool Workshop"]')).toBeExisting()
    await expect($('button=Overview')).toBeExisting()
    await expect($('button=Queues')).toBeExisting()
  })

  it('What-if offers shortage building types and a runnable control', async () => {
    await openWorkspaceTab('What-if')
    await expect($('#what-if-heading')).toHaveText('What-if scenario')
    const building = await $('select[aria-label="Building"], input[aria-label="Building"]')
    await expect(building).toBeExisting()
    const tag = await building.getTagName()
    if (tag.toLowerCase() === 'select') {
      const options = await building.$$('option')
      const labels = []
      for (const option of options) {
        labels.push(await option.getText())
      }
      const joined = labels.join(' ').toLowerCase()
      if (!joined.includes('lumber') || !joined.includes('tool')) {
        throw new Error(`what-if building options missing mock types: ${labels.join(', ')}`)
      }
    } else {
      await expect($('*=building_mock')).toBeExisting()
    }
    await expect($('input[aria-label="Extra Levels"]')).toBeExisting()
    const run = await $('button=Run what-if')
    await expect(run).toBeExisting()
    await expect(run).toBeEnabled()
  })

  it('Timeline shows the plan form with goods from the solve', async () => {
    await openWorkspaceTab('Timeline')
    await expect($('#timeline-tool-heading')).toHaveText('Plan timeline')
    await expect($('button=Build timeline')).toBeExisting()
    await expect($('input[aria-label="Plan label"]')).toBeExisting()
    const page = await $('section[aria-labelledby="timeline-tool-heading"]')
    const text = (await page.getText()).toLowerCase()
    if (!text.includes('mock_lumber') && !text.includes('lumber') && !text.includes('goal')) {
      throw new Error(`timeline pane missing goal/goods chrome: ${text.slice(0, 240)}`)
    }
  })

  it('Goal gaps shows readiness controls', async () => {
    await openWorkspaceTab('Goal gaps')
    await expect($('#gaps-form-heading')).toHaveText('Goal gaps')
    await expect($('button=Check readiness')).toBeExisting()
  })

  it('Query shows SQL editor on desktop or the web-unavailable notice', async () => {
    await openWorkspaceTab('Query')
    await expect($('#query-heading')).toHaveText('Advanced SQL Queries')
    if (isTauriE2e()) {
      await expect($('#sql-editor')).toBeExisting()
      await expect($('#run-sql')).toBeExisting()
    } else {
      await expect($('*=Tauri Desktop App Required')).toBeExisting()
    }
  })

  it('Archive shows import controls and empty or listed records', async () => {
    await openWorkspaceTab('Archive')
    await expect($('#archive-heading')).toHaveText('Past saves')
    await expect($('input[aria-label="Import AnalysisRecord"]')).toBeExisting()
    await expect($('*=Select two analyses to compare')).toBeExisting()
    const empty = await $('*=No archived analyses yet')
    const list = await $('.archive-list')
    if (!(await empty.isExisting()) && !(await list.isExisting())) {
      throw new Error('Archive pane has neither empty copy nor a record list')
    }
  })

  it('desktop Saves lists fixture .v3 rows; Settings shows path fields', async function () {
    if (!isTauriE2e()) {
      this.skip()
    }
    await $('button*=Saves').click()
    await expect($('[aria-label="Desktop save catalog"]')).toBeExisting()
    await expect($('td*=mock_shortage.v3')).toBeExisting()
    await expect($('#refresh-saves')).toBeExisting()

    await $('button*=Settings').click()
    await expect($('#settings-heading')).toHaveText('Settings')
    await expect($('#cfg-game')).toBeExisting()
    await expect($('#cfg-saves')).toBeExisting()
    const gameVal = await $('#cfg-game').getValue()
    if (!String(gameVal).includes('mock_game')) {
      throw new Error(`expected cfg-game to point at mock_game, got ${gameVal}`)
    }
  })

  it('web still exposes the save upload zone after load', async function () {
    if (!isWebE2e()) {
      this.skip()
    }
    await expect($('.inputs')).toBeExisting()
    await expect($('input[aria-label="Save file"]')).toBeExisting()
    await expect($('input[aria-label="Definitions file"]')).toBeExisting()
  })
})
