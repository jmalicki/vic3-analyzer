import { spawn } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { chromium } from 'playwright'
import {
  WEB_SHOTS,
  outDir,
  requireDefsBlob,
  requireWasm,
  saveFixture,
  webRoot,
} from './lib/paths.mjs'
import { seedDefsIndexedDb } from './lib/seed-web.mjs'
import { newShotPage, writeShot } from './lib/shot.mjs'

const BASE = '/vic3-analyzer/'
const PREVIEW_PORT = Number(process.env.DOCS_WEB_PREVIEW_PORT || 4173)

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

async function waitForUrl(url, attempts = 60) {
  for (let i = 0; i < attempts; i++) {
    try {
      const res = await fetch(url)
      if (res.ok || res.status === 404) return
    } catch {
      // not up yet
    }
    await sleep(500)
  }
  throw new Error(`Timed out waiting for ${url}`)
}

async function startPreview() {
  const child = spawn(
    'npx',
    ['vite', 'preview', '--host', '127.0.0.1', '--port', String(PREVIEW_PORT), '--strictPort'],
    {
      cwd: webRoot,
      stdio: ['ignore', 'pipe', 'pipe'],
      env: { ...process.env },
      detached: true,
    },
  )
  let logs = ''
  child.stdout?.on('data', (chunk) => {
    logs += chunk.toString()
  })
  child.stderr?.on('data', (chunk) => {
    logs += chunk.toString()
  })
  const baseUrl = `http://127.0.0.1:${PREVIEW_PORT}${BASE}`
  try {
    await waitForUrl(baseUrl)
  } catch (err) {
    child.kill('SIGTERM')
    throw new Error(`${err.message}\nPreview logs:\n${logs}`)
  }
  return {
    baseUrl,
    stop: async () => {
      if (!child.pid) return
      try {
        process.kill(-child.pid, 'SIGKILL')
      } catch {
        try {
          child.kill('SIGKILL')
        } catch {
          // already gone
        }
      }
    },
  }
}

/** @param {import('playwright').Page} page */
async function waitReady(page) {
  console.log('waiting for campaign summary / goods prices…')
  await page.getByRole('region', { name: 'Campaign summary' }).waitFor({ timeout: 90_000 })
  await page.getByRole('heading', { name: 'Goods prices' }).waitFor({ timeout: 90_000 })
  console.log('prices ready')
}

/** @param {import('playwright').Page} page */
async function goView(page, label) {
  await page.getByRole('navigation', { name: 'Analysis tools' }).getByRole('button', { name: label }).click()
}

async function main() {
  requireWasm()
  requireDefsBlob()

  const dest = outDir()
  const preview = await startPreview()
  const browser = await chromium.launch({ headless: true })
  try {
    console.log('web capture: seeding defs…')
    const { context, page } = await newShotPage(browser)

    await page.goto(preview.baseUrl, { waitUntil: 'domcontentloaded', timeout: 60_000 })
    await seedDefsIndexedDb(page)
    console.log('web capture: reloading with seeded defs…')
    await page.reload({ waitUntil: 'domcontentloaded', timeout: 60_000 })
    await page.getByText(/Using your file: defs\.postcard/).waitFor({ timeout: 60_000 })

    console.log('web capture: loading save…')
    await page.getByLabel('Save file').setInputFiles({
      name: 'plaintext.v3',
      mimeType: 'text/plain',
      buffer: readFileSync(saveFixture),
    })
    await waitReady(page)

    // W1 Prices
    await writeShot(page, dest, WEB_SHOTS[0])

    // W2 Good drill-down
    const goodLink = page.locator('a.good-link').first()
    await goodLink.waitFor({ timeout: 30_000 })
    await goodLink.click()
    await page.waitForURL(/#\/prices\/good\//)
    await sleep(400)
    await writeShot(page, dest, WEB_SHOTS[1])

    // W5 Buildings overview first (needed to discover building ids), then W3 detail
    await goView(page, 'Buildings')
    await page.getByRole('heading', { name: 'Buildings', exact: true }).waitFor({ timeout: 30_000 })
    await sleep(400)
    await writeShot(page, dest, WEB_SHOTS[4])

    // W3 Building detail — expand a type row, then open an instance
    const expandBtn = page.locator('button.building-expand').first()
    await expandBtn.waitFor({ timeout: 30_000 })
    await expandBtn.click()
    const buildingLink = page.locator('a.building-link').first()
    await buildingLink.waitFor({ timeout: 30_000 })
    await buildingLink.click()
    await page.waitForURL(/#\/buildings\/building\//)
    await sleep(500)
    await writeShot(page, dest, WEB_SHOTS[2])

    // W4 What-if
    await goView(page, 'What-if')
    await page.getByRole('heading', { name: 'What-if scenario' }).waitFor({ timeout: 30_000 })
    await sleep(300)
    await writeShot(page, dest, WEB_SHOTS[3])

    // W6 Alerts
    await goView(page, 'Alerts')
    await page.getByRole('heading', { name: 'Alerts', exact: true }).waitFor({ timeout: 30_000 })
    await sleep(500)
    await writeShot(page, dest, WEB_SHOTS[5])

    // W7a Goal gaps — Prepare for war
    await goView(page, 'Goal gaps')
    await page.getByLabel('Gaps default plan').selectOption('war-readiness')
    await page.getByRole('button', { name: 'Check readiness' }).click()
    await page.locator('#gaps-heading').waitFor({ timeout: 120_000 })
    await sleep(400)
    await writeShot(page, dest, WEB_SHOTS[6])

    // W7b Timeline — Grow the economy
    await goView(page, 'Timeline')
    await page.getByLabel('Plan default plan').selectOption('economic-growth')
    await page.getByRole('button', { name: 'Build timeline' }).click()
    // Plan may succeed or show an error on tiny fixture — still capture the workspace.
    await Promise.race([
      page.locator('#plan-heading').waitFor({ timeout: 120_000 }),
      page.getByRole('alert').waitFor({ timeout: 120_000 }),
      sleep(15_000),
    ])
    await sleep(400)
    await writeShot(page, dest, WEB_SHOTS[7])

    // W8 Archive
    await goView(page, 'Archive')
    await page.getByRole('heading', { name: 'Past saves' }).waitFor({ timeout: 30_000 })
    await sleep(300)
    await writeShot(page, dest, WEB_SHOTS[8])

    // W9 Defs builder modal
    await page.getByRole('button', { name: /Build definitions from game files/ }).click()
    await page.getByRole('heading', { name: /Build definitions/i }).waitFor({ timeout: 30_000 })
    await sleep(300)
    await writeShot(page, dest, WEB_SHOTS[9])

    console.log('web capture: done, closing…')
    await context.close()
  } finally {
    try {
      await Promise.race([
        (async () => {
          await browser.close()
          await preview.stop()
        })(),
        sleep(5_000),
      ])
    } catch (err) {
      console.warn('web capture cleanup:', err)
    }
  }
  // Vite preview / Playwright can leave handles open; force exit after success.
  process.exit(0)
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
