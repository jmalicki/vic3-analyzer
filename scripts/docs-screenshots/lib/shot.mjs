import { mkdirSync } from 'node:fs'
import { join } from 'node:path'

export const VIEWPORT = { width: 1280, height: 800 }

/** Crisp embeds for markdown (matches plan: 1280×800 @ 2x). */
export const DEVICE_SCALE = 2

/**
 * @param {import('playwright').Page} page
 * @param {string} dir
 * @param {string} filename
 * @param {{ selector?: string }} [opts]
 */
export async function writeShot(page, dir, filename, opts = {}) {
  mkdirSync(dir, { recursive: true })
  const path = join(dir, filename)
  const selector = opts.selector
  if (selector) {
    const el = page.locator(selector)
    await el.waitFor({ state: 'visible', timeout: 30_000 })
    await el.screenshot({ path, type: 'png' })
  } else {
    await page.screenshot({ path, fullPage: false, type: 'png' })
  }
  console.log(`wrote ${path}`)
  return path
}

/**
 * Wrap the companion UI in a faux macOS window chrome so docs shots read as a
 * native app (mock Chromium has no OS frame; real Tauri WDIO can replace later).
 *
 * @param {import('playwright').Page} page
 * @param {{ title?: string }} [opts]
 */
export async function installMacWindowChrome(page, opts = {}) {
  const title = opts.title || 'Vic3 Analyzer'
  await page.addStyleTag({
    content: `
      html, body {
        margin: 0 !important;
        padding: 0 !important;
        min-height: 100% !important;
        background: #6b7380 !important;
        background-image: none !important;
      }
      #docs-mac-window {
        box-sizing: border-box;
        width: calc(100vw - 48px);
        height: calc(100vh - 48px);
        margin: 24px;
        display: flex;
        flex-direction: column;
        border-radius: 10px;
        overflow: hidden;
        background: #f3f6f9;
        box-shadow:
          0 0 0 1px rgba(0, 0, 0, 0.28),
          0 18px 50px rgba(0, 0, 0, 0.35);
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      }
      #docs-mac-titlebar {
        flex: 0 0 38px;
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 0 14px;
        background: linear-gradient(180deg, #eceff3 0%, #d9dee6 100%);
        border-bottom: 1px solid rgba(0, 0, 0, 0.18);
        user-select: none;
      }
      #docs-mac-traffic {
        display: flex;
        gap: 8px;
        flex: 0 0 auto;
      }
      #docs-mac-traffic span {
        width: 12px;
        height: 12px;
        border-radius: 50%;
        box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.18);
      }
      #docs-mac-traffic .close { background: #ff5f57; }
      #docs-mac-traffic .min { background: #febc2e; }
      #docs-mac-traffic .zoom { background: #28c840; }
      #docs-mac-title {
        flex: 1 1 auto;
        text-align: center;
        font-size: 13px;
        font-weight: 600;
        color: #1c2430;
        letter-spacing: -0.01em;
        margin-right: 56px; /* balance traffic lights */
      }
      #docs-mac-content {
        flex: 1 1 auto;
        overflow: auto;
        background:
          radial-gradient(1200px 600px at 10% -10%, #c5d4e4 0%, transparent 55%),
          linear-gradient(165deg, #f3f6f9 0%, #d9e3ec 100%);
      }
      #docs-mac-content > body,
      #docs-mac-content {
        /* companion styles expect padding on body; restore on content */
      }
      #docs-mac-content .docs-mac-inner {
        padding: 1.5rem;
        box-sizing: border-box;
        min-height: 100%;
      }
    `,
  })
  await page.evaluate((windowTitle) => {
    if (document.getElementById('docs-mac-window')) return
    const shell = document.createElement('div')
    shell.id = 'docs-mac-window'
    shell.innerHTML = `
      <div id="docs-mac-titlebar">
        <div id="docs-mac-traffic" aria-hidden="true">
          <span class="close"></span><span class="min"></span><span class="zoom"></span>
        </div>
        <div id="docs-mac-title"></div>
      </div>
      <div id="docs-mac-content"><div class="docs-mac-inner"></div></div>
    `
    shell.querySelector('#docs-mac-title').textContent = windowTitle
    const inner = shell.querySelector('.docs-mac-inner')
    while (document.body.firstChild) {
      inner.appendChild(document.body.firstChild)
    }
    document.body.appendChild(shell)
  }, title)
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
