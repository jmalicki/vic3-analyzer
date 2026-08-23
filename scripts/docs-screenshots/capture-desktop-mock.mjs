import { createServer } from 'node:http'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import handler from 'serve-handler'
import { chromium } from 'playwright'
import {
  DESKTOP_SHOTS,
  desktopUiDir,
  outDir,
  packageRoot,
  saveFixture,
} from './lib/paths.mjs'
import { newShotPage, writeShot } from './lib/shot.mjs'
import { seedDefsIndexedDb } from './lib/seed-web.mjs'

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

const fixture = JSON.parse(
  readFileSync(join(packageRoot, 'fixtures/desktop-mock-data.json'), 'utf8'),
)

/**
 * Install window.__TAURI__ before the companion script runs.
 * @param {import('playwright').Page} page
 */
async function installTauriMock(page) {
  await page.addInitScript((data) => {
    const queries = data.queries
    function pick(sql) {
      const s = String(sql).toLowerCase()
      if (s.includes('player_tag') || s.includes('army_power')) return queries.basic
      if (s.includes('from alerts') && s.includes('group by kind, severity')) return queries.top_alerts
      if (
        s.includes('from alerts') ||
        s.includes('suggest_mitigations') ||
        s.includes('mitigation')
      ) {
        return queries.alerts_mitigations
      }
      if (s.includes('shortage') || (s.includes('from goods') && s.includes('shortage'))) {
        return queries.shortage
      }
      if (s.includes('from states')) return queries.states
      if (s.includes('from goods where') || s.includes("good =")) return queries.prices
      if (s.includes('plan(')) return queries.plan
      if (s.includes('from saves')) return queries.saves
      return queries.default
    }

    let loaded = data.dashboard.loaded_stub

    window.__TAURI__ = {
      core: {
        invoke: async (cmd, args = {}) => {
          switch (cmd) {
            case 'get_dashboard':
              return {
                config: data.config,
                game_detected: data.dashboard.game_detected,
                save_root_count: data.dashboard.save_root_count,
                save_count: data.dashboard.save_count,
                loaded_stub: loaded,
                detection_hints: data.dashboard.detection_hints,
              }
            case 'read_save_bytes':
              return window.__MOCK_SAVE_BYTES__
            case 'list_saves':
              return data.queries.saves.rows.map(row => ({
                name: row[0],
                kind: row[1],
                mtime: 1672531200000,
                location: 'local'
              }))
            case 'use_save':
              loaded = args.name || loaded
              return JSON.stringify(data.use_save)
            case 'sql_query':
              return JSON.stringify(pick(args.sql || ''))
            case 'sql_docs':
              return data.sql_docs
            case 'loaded_prices':
              return data.loaded_prices
            case 'loaded_alerts':
              return data.loaded_alerts
            case 'detection_hints':
              return data.dashboard.detection_hints
            case 'save_config':
              return { ...data.config, ...(args.config || {}) }
            case 'reset_config':
              return data.config
            case 'get_config':
              return data.config
            default:
              throw new Error(`docs-screenshots mock: unhandled invoke ${cmd}`)
          }
        },
      },
      event: {
        listen: async () => () => {},
      },
    }
  }, fixture)
}

async function startStaticServer(root) {
  const server = createServer((request, response) => {
    if (request.url.startsWith('/vic3-analyzer/')) {
      request.url = request.url.substring('/vic3-analyzer'.length)
    }
    return handler(request, response, { public: root })
  })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  const { port } = server.address()
  return {
    url: `http://127.0.0.1:${port}/index.html`,
    close: () => new Promise((resolve, reject) => server.close((err) => (err ? reject(err) : resolve()))),
  }
}

async function main() {
  const dest = outDir()
  const server = await startStaticServer(desktopUiDir)
  const browser = await chromium.launch({ headless: true })
  try {
    const { context, page } = await newShotPage(browser)
    page.on('console', msg => console.log('PAGE LOG:', msg.text()))
    await installTauriMock(page)
    await page.addInitScript((bytes) => {
      window.__MOCK_SAVE_BYTES__ = bytes
    }, Array.from(readFileSync(saveFixture)))
    await page.goto(server.url, { waitUntil: 'domcontentloaded', timeout: 60_000 })
    await seedDefsIndexedDb(page)
    await page.reload({ waitUntil: 'domcontentloaded', timeout: 60_000 })
    await page.waitForSelector('.saves-table tbody tr td', { timeout: 30_000 })
    await page.waitForFunction(() => {
      const text = document.querySelector('.defs-required')?.textContent || ''
      return !text.includes('definitions') && !text.includes('Loading')
    }, { timeout: 30_000 })
    await page.click('.saves-table tbody tr td')
    await page.waitForSelector('.active-save-bar', { timeout: 30_000 })

    // D1 Dashboard
    await writeShot(page, dest, DESKTOP_SHOTS[0])

    // D2 Saves
    await page.getByRole('button', { name: 'Change Save' }).click()
    await page.waitForSelector('.saves-table')
    await writeShot(page, dest, DESKTOP_SHOTS[1])
    await page.getByRole('button', { name: 'Cancel' }).click()

    // D3 Alert mitigations
    await page.click('button:has-text("Alerts")')
    await page.waitForSelector('.alert-groups', { timeout: 30_000 })
    await writeShot(page, dest, DESKTOP_SHOTS[2])

    // D4 Advanced Query + shortage SQL
    await page.getByRole('navigation', { name: 'Analysis tools' }).getByRole('button', { name: 'Query' }).click()
    await page.click('button[data-ex="alerts"]')
    await page.click('#run-sql')
    await sleep(2000)
    await page.waitForSelector('#results-body tr', { timeout: 5000 })
    await writeShot(page, dest, DESKTOP_SHOTS[3])

    // D5 States
    await page.getByRole('navigation', { name: 'Analysis tools' }).getByRole('button', { name: 'States' }).click()
    await sleep(300)
    await writeShot(page, dest, DESKTOP_SHOTS[4])

    // D6 Prices
    await page.getByRole('navigation', { name: 'Analysis tools' }).getByRole('button', { name: 'Prices' }).click()
    await sleep(300)
    await writeShot(page, dest, DESKTOP_SHOTS[5])

    // D7 Timeline
    await page.getByRole('navigation', { name: 'Analysis tools' }).getByRole('button', { name: 'Timeline' }).click()
    await page.waitForSelector('.guided-form', { timeout: 30_000 })
    await writeShot(page, dest, DESKTOP_SHOTS[6])

    console.log('desktop mock: done')
    await context.close()
  } finally {
    try {
      await Promise.race([
        (async () => {
          await browser.close()
          await server.close()
        })(),
        new Promise((r) => setTimeout(r, 5_000)),
      ])
    } catch (err) {
      console.warn('desktop mock cleanup:', err)
    }
  }
  process.exit(0)
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
