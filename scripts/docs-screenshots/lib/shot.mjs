import { mkdirSync } from 'node:fs'
import { join } from 'node:path'

export const VIEWPORT = { width: 1280, height: 800 }

/** Crisp embeds for markdown (matches plan: 1280×800 @ 2x). */
export const DEVICE_SCALE = 2

/**
 * @param {import('playwright').Page} page
 * @param {string} dir
 * @param {string} filename
 */
export async function writeShot(page, dir, filename) {
  mkdirSync(dir, { recursive: true })
  const path = join(dir, filename)
  await page.screenshot({ path, fullPage: false, type: 'png' })
  console.log(`wrote ${path}`)
  return path
}

/**
 * @param {import('playwright').Browser} browser
 */
export async function newShotPage(browser) {
  const context = await browser.newContext({
    viewport: VIEWPORT,
    deviceScaleFactor: DEVICE_SCALE,
  })
  const page = await context.newPage()
  return { context, page }
}
