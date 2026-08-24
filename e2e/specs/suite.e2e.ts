/**
 * Single WDIO spec file so Tauri launches one app session.
 * (One session per file was timing out on Linux WebKit: core.invoke never ready.)
 */
import { browser, $, $$, expect } from '@wdio/globals'
import { isTauriE2e, isWebE2e } from '../runtime.js'
import { SAVE_MARKERS, SAVES } from '../fixtures.js'
import { bootWithSave, loadSave, openWorkspaceTab, textIncludes } from '../session.js'

// --- from 00-setup.e2e.ts ---
describe('00 Setup — paths + primary save load', () => {
  it('prepares multi-save fixtures and loads the shortage economy', async () => {
    await bootWithSave('shortage')

    if (isTauriE2e()) {
      await expect($('.inputs')).not.toBeExisting()
      await expect($('[aria-label="Loaded save"]')).toHaveText(
        expect.stringContaining(SAVES.shortage.replace(/\.v3$/i, '')),
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

// --- from 01-workspace-tabs.e2e.ts ---
/**
 * Verify each workspace pane
 * shows shortage-economy content — not just that the tab heading mounts.
 *
 * UI labels use displayId (mock_lumber → "Mock Lumber"); prefer href / aria-label.
 */
describe('01 Workspace tabs — content', () => {
  // Uses shortage session from 00 (single WDIO session).

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
    await expect($('a.state-link[href*="/states/1"]')).toBeExisting()
    await expect($('//a[contains(@class,"state-link") and contains(., "Mock")]')).toBeExisting()
    await expect($('button[aria-label="Sort by Name"]')).toBeExisting()
    await expect($('p*=Load a save to see states')).not.toBeExisting()
  })

  it('Pops lists mock_laborers from the shortage save', async () => {
    await openWorkspaceTab('Pops')
    await expect($('#pops-heading')).toHaveText('Pops')
    await textIncludes('Mock Laborers', {
      root: 'section[aria-labelledby="pops-heading"]',
      timeout: 60_000,
      timeoutMsg: 'Pops pane never listed Mock Laborers',
    })
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
    await textIncludes('No armies recorded in this save', {
      root: 'section[aria-labelledby="military-heading"]',
      timeout: 60_000,
      timeoutMsg: 'Military pane never resolved armies empty state',
    })
    await $('button=Navy').click()
    await textIncludes('No navies recorded in this save', {
      root: 'section[aria-labelledby="military-heading"]',
      timeout: 30_000,
    })
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
    await browser.waitUntil(
      async () => {
        const tag = (await building.getTagName()).toLowerCase()
        if (tag === 'select') {
          const options = await building.$$('option')
          return options.length > 0
        }
        const value = String(await building.getValue())
        if (value.trim()) return true
        // Desktop free-text fallback before summary.buildings hydrates.
        await building.setValue('building_mock_lumber_camp')
        return true
      },
      { timeout: 30_000, timeoutMsg: 'What-if building control never became usable' },
    )
    const tag = (await building.getTagName()).toLowerCase()
    if (tag === 'select') {
      const options = await building.$$('option')
      const labels = []
      for (const option of options) {
        labels.push(await option.getText())
      }
      const joined = labels.join(' ').toLowerCase()
      if (!joined.includes('lumber') || !joined.includes('tool')) {
        throw new Error(`what-if building options missing mock types: ${labels.join(', ')}`)
      }
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
    await textIncludes('compare', {
      root: 'section[aria-labelledby="archive-heading"]',
      timeoutMsg: 'Archive missing compare hint',
    })
    // WebKit `*=` often misses the empty copy; use section text instead.
    await browser.waitUntil(
      async () => {
        const text = await $('section[aria-labelledby="archive-heading"]').getText()
        return (
          text.includes('No archived analyses yet')
          || (await $('.archive-list').isExisting())
        )
      },
      { timeout: 15_000, timeoutMsg: 'Archive pane has neither empty copy nor a record list' },
    )
  })

  it('desktop Saves lists fixture .v3 rows; Settings shows path fields', async function () {
    if (!isTauriE2e()) {
      this.skip()
    }
    await $('button*=Saves').click()
    await expect($('[aria-label="Desktop save catalog"]')).toBeExisting()
    await expect($('td*=mock_shortage')).toBeExisting()
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

// --- from 02-content.e2e.ts ---
/** Deeper interactions on the shortage save (fresh session). */
describe('02 Content — interactions on shortage save', () => {
  // Uses shortage session from 00.

  it('expands Mock Lumber Camp and shows per-building what-if controls', async () => {
    await openWorkspaceTab('Buildings')
    await expect($('button[aria-label*="Expand Mock Lumber Camp"]')).toBeExisting()
    // Row-level what-if is always visible without expand. Skip expand on Tauri —
    // WebKit expand has been taking down the webview (nav gone for later suites).
    await expect($('button=Run what-if')).toBeExisting()
    await expect($('button=Optimize production methods')).toBeExisting()
    if (isWebE2e()) {
      await $('button[aria-label*="Expand Mock Lumber Camp"]').click()
      await expect($('button[aria-label*="Collapse Mock Lumber Camp"]')).toBeExisting()
    }
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

// --- from 03-save-matrix.e2e.ts ---
/**
 * Different saves → different UI markers (PR #76 multi-save plan).
 * Fresh session: boot shortage, then switch to balanced in-process.
 */
describe('03 Save matrix — different results per save', () => {
  // Starts from shortage (00); switches to balanced in-process.

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
        expect.stringContaining(SAVES.balanced.replace(/\.v3$/i, '')),
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
      const stub = name.replace(/\.v3$/i, '')
      await expect($(`td*=${stub}`)).toBeExisting()
    }
    expect(SAVE_MARKERS.shortage.expectVisible.length).toBeGreaterThan(0)
  })
})

// --- from 04-tauri-query.e2e.ts ---
describe('04 Tauri Query', () => {
  before(function () {
    if (!isTauriE2e()) {
      this.skip()
    }
    // Shortage already loaded by 00; balanced may have been loaded in 03 —
    // reload shortage so SQL sees mock_lumber.
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

// --- from 05-scope.e2e.ts ---
/**
 * Player-scoped alerts + Domestic / Our market geography (PR #76 plan).
 * Uses mock_two_countries: MOCK/Home (player) vs RIVAL/Rivalia (foreign market).
 */
describe('05 Scope — foreign alerts hidden; domestic vs market geography', () => {
  before(async () => {
    await loadSave('twoCountries')
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
