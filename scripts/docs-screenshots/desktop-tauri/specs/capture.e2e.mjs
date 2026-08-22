import { join } from 'node:path'
import { DESKTOP_SHOTS, outDir } from '../../lib/paths.mjs'
import { captureMacWindow } from '../mac-window-capture.mjs'

const dest = process.env.DOCS_SCREENSHOTS_OUT || outDir()

async function click(sel) {
  const el = await $(sel)
  await el.waitForExist({ timeout: 30_000 })
  await el.click()
}

async function waitFor(sel) {
  const el = await $(sel)
  await el.waitForExist({ timeout: 60_000 })
}

async function shot(name) {
  // Let paint settle after navigation.
  await browser.pause(400)
  const path = join(dest, name)
  captureMacWindow(path)
  console.log('wrote', path)
}

describe('desktop docs screenshots (native macOS chrome)', () => {
  it('captures locked desktop-*.png set', async () => {
    await waitFor('#metrics .metric')
    await shot(DESKTOP_SHOTS[0])

    await click('#tab-saves')
    await waitFor('#saves-body tr')
    await shot(DESKTOP_SHOTS[1])

    // Load fixture save so Alerts / Query have real data.
    await click('#saves-body tr[data-stub]')
    await browser.pause(2000)

    await click('#tab-alerts')
    await waitFor('#alerts-root')
    // Real fixture may have zero alerts; still capture the pane.
    await browser.pause(800)
    await shot(DESKTOP_SHOTS[2])

    await click('#tab-query')
    await click('button[data-ex="shortage"]')
    await click('#run-sql')
    await waitFor('#results-body td')
    await shot(DESKTOP_SHOTS[3])

    const stateCell = await $('#results-body td.nav-key[data-col="state_id"]')
    if (await stateCell.isExisting()) {
      await stateCell.click()
      await waitFor('#view-states:not(.hidden)')
      await waitFor('#states-body td')
      await shot(DESKTOP_SHOTS[4])
    } else {
      await click('#tab-states')
      await shot(DESKTOP_SHOTS[4])
    }

    await click('#tab-query')
    await click('button[data-ex="shortage"]')
    await click('#run-sql')
    await waitFor('#results-body td')
    const goodCell = await $('#results-body td.nav-key[data-col="good"]')
    if (await goodCell.isExisting()) {
      await goodCell.click()
      await waitFor('#view-prices:not(.hidden)')
      await shot(DESKTOP_SHOTS[5])
    } else {
      await click('#tab-prices')
      await shot(DESKTOP_SHOTS[5])
    }

    await click('#tab-query')
    const editor = await $('#sql-editor')
    await editor.setValue(
      "SELECT step, day, action, detail FROM plan('gdp >= 100000000') ORDER BY step;",
    )
    await click('#run-sql')
    await browser.pause(1000)
    const stepCell = await $('#results-body td.nav-key[data-col="step"]')
    if (await stepCell.isExisting()) {
      await stepCell.click()
      await waitFor('#view-timeline:not(.hidden)')
      await shot(DESKTOP_SHOTS[6])
    } else {
      await click('#tab-timeline')
      await shot(DESKTOP_SHOTS[6])
    }

    await click('#tab-settings')
    await waitFor('#view-settings:not(.hidden)')
    await shot(DESKTOP_SHOTS[7])
  })
})
