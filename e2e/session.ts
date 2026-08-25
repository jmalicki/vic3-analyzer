import { browser, $, $$, expect } from '@wdio/globals'
import { join } from 'node:path'
import { isTauriE2e, isWebE2e } from './runtime.js'
import {
  e2eSavesDir,
  mockGameDefsPostcard,
  mockGameDir,
  prepareE2eSaves,
  SAVES,
  type SaveKey,
} from './fixtures.js'

const ANALYSIS_TIMEOUT = 120_000

/**
 * WDIO starts a fresh browser/app session per spec file. Always open the app
 * (and optionally load a save) in each suite — do not rely on 00-setup state.
 */
export async function ensureAppOpen(): Promise<void> {
  if (isWebE2e()) {
    await browser.url('')
  }
  await expect($('body')).toBeExisting()
  // Linux WebKit / cold Tauri can take well over 15s before the title is ready.
  await browser.waitUntil(
    async () => (await browser.getTitle()).includes('Victoria 3 Analyzer'),
    {
      timeout: isTauriE2e() ? 120_000 : 15_000,
      timeoutMsg: 'title never contained Victoria 3 Analyzer',
    },
  )
}

/** Prepare fixtures, open the app, configure desktop paths, load a save. */
export async function bootWithSave(saveKey: SaveKey): Promise<void> {
  prepareE2eSaves()
  await ensureAppOpen()
  if (isTauriE2e()) {
    await configureTauriPaths()
  }
  await loadSave(saveKey)
}

export async function openWorkspaceTab(label: string): Promise<void> {
  // Chain CSS + WDIO text selectors — a single "nav … button*=" string is
  // invalid CSS and Chromium rejects it (BiDi + classic).
  const nav = await $('nav[aria-label="Analysis tools"]')
  await expect(nav).toBeExisting()
  const tab = await nav.$(`button*=${label}`)
  await expect(tab).toBeExisting()
  await tab.click()
}

/** Body/section text check — more reliable than WebKit `*=` partial selectors. */
export async function textIncludes(
  needle: string,
  opts?: { root?: string; timeout?: number; timeoutMsg?: string },
): Promise<void> {
  const root = opts?.root
  const timeout = opts?.timeout ?? 30_000
  const lowered = needle.toLowerCase()
  await browser.waitUntil(
    async () => {
      const text = root
        ? await $(root).getText().catch(() => '')
        : await $('body').getText().catch(() => '')
      return String(text).toLowerCase().includes(lowered)
    },
    {
      timeout,
      timeoutMsg: opts?.timeoutMsg ?? `never saw text containing "${needle}"`,
    },
  )
}

/** Wait until analysis is priced (campaign HUD + at least one goods row). */
export async function waitForAnalysisReady(): Promise<void> {
  // Goods links only mount on the Prices list view — switch there first so
  // mid-suite save reloads (e.g. Buildings → load balanced) still converge.
  // Desktop Load already calls selectView('prices') when use_save finishes.
  const nav = await $('nav[aria-label="Analysis tools"]')
  if (await nav.isExisting()) {
    const prices = await nav.$('button*=Prices')
    if (await prices.isExisting()) {
      await prices.click()
    }
  }

  await browser.waitUntil(
    async () => {
      const hud = await $('section[aria-label="Campaign summary"]')
      if (!(await hud.isExisting())) return false
      const empty = await $('p*=Prices appear after a save is priced')
      if (await empty.isExisting()) return false
      // Heading alone is not enough (chrome mounts before rows). Goods use
      // displayId labels ("Mock Lumber"); href keeps the script id.
      const links = await $$('a.good-link')
      return links.length > 0
    },
    {
      timeout: ANALYSIS_TIMEOUT,
      timeoutMsg: 'Analysis never became ready (HUD + goods links)',
    },
  )
}

export async function loadWebSave(saveKey: SaveKey): Promise<void> {
  const defsInput = await $('input[aria-label="Definitions file"]')
  await expect(defsInput).toBeExisting()
  // Chromium BiDi sometimes drops the first file-input change; set twice.
  await defsInput.setValue(mockGameDefsPostcard)
  await defsInput.setValue(mockGameDefsPostcard)
  await browser.waitUntil(
    async () => {
      const body = await $('body').getText().catch(() => '')
      return body.includes('Using your file') && body.includes('mock_game.defs')
    },
    { timeout: 30_000, timeoutMsg: 'Definitions file never applied' },
  )

  const savePath = join(e2eSavesDir, SAVES[saveKey])
  const saveInput = await $('input[aria-label="Save file"]')
  await expect(saveInput).toBeExisting()
  // Chromedriver often skips change events when setValue is repeated on the
  // same file input — clear first so the second save always reloads.
  await browser.execute(() => {
    const input = document.querySelector('input[aria-label="Save file"]') as HTMLInputElement | null
    if (input) {
      input.value = ''
      input.dispatchEvent(new Event('change', { bubbles: true }))
    }
  })
  await saveInput.setValue(savePath)
  await browser.waitUntil(
    async () => {
      const text = await $('.inputs').getText().catch(() => '')
      return text.includes(SAVES[saveKey])
    },
    { timeout: 30_000, timeoutMsg: `Save file ${SAVES[saveKey]} never appeared in the upload UI` },
  )
  await waitForAnalysisReady()
}

export async function configureTauriPaths(): Promise<void> {
  const settingsTab = await $('button*=Settings')
  await settingsTab.click()

  // Wait for get_config to finish — otherwise it overwrites setValue and we
  // save blank/auto-detect paths (catalog then misses fixture .v3 files).
  await browser.waitUntil(
    async () => {
      const pathEl = await $('#cfg-path')
      return (await pathEl.isExisting()) && (await pathEl.getText()).trim().length > 0
    },
    { timeout: 60_000, timeoutMsg: 'Settings never loaded config_path from get_config' },
  )

  await $('#cfg-game').setValue(mockGameDir)
  await $('#cfg-saves').setValue(e2eSavesDir)
  await $('#cfg-defs').setValue(mockGameDefsPostcard)

  const auto = await $('#cfg-auto')
  if (await auto.isSelected()) {
    await auto.click()
  }

  await $('#save-settings').click()
  await expect($('#settings-status')).toHaveText(expect.stringContaining('Saved'))
  await expect($('#cfg-game')).toHaveValue(expect.stringContaining('mock_game'))
  await expect($('#cfg-saves')).toHaveValue(expect.stringContaining('e2e_saves'))
  await expect($('#cfg-defs')).toHaveValue(expect.stringContaining('mock_game.defs.postcard'))
}

type Vic3WdioDiag = {
  nativeInvokeCaptured: boolean
  invokeIsAccessor: boolean
  mocksKeys: string[]
  interceptorFlag: boolean
  strippedInterception: boolean
  probeCurrent: (cmd: string, args?: Record<string, unknown>) => Promise<WdioInvokeProbe>
  probeNative: (cmd: string, args?: Record<string, unknown>) => Promise<WdioInvokeProbe>
}

type WdioInvokeProbe = {
  ok: boolean
  ms: number
  resultType?: string
  error?: string
}

type WdioLoadDiag = {
  meta?: {
    nativeInvokeCaptured: boolean
    invokeIsAccessor: boolean
    mocksKeys: string[]
    interceptorFlag: boolean
    strippedInterception: boolean
  }
  listSavesCurrent?: WdioInvokeProbe
  listSavesNative?: WdioInvokeProbe
  useSaveCurrent?: WdioInvokeProbe
  useSaveNative?: WdioInvokeProbe
  saveTrace?: unknown
  resolvedLocation?: string
}

/** Snapshot guest WDIO / invoke state; optionally try use_save via current + native paths. */
async function collectTauriLoadDiag(
  stub: string,
  opts: { tryUseSave: boolean },
): Promise<WdioLoadDiag> {
  return browser.execute(
    async (stubName, tryUseSave) => {
      const w = window as Window & {
        __vic3_wdio_diag__?: Vic3WdioDiag
        __vic3_desktop_save_trace__?: {
          phase?: string
          location?: string
          error?: string
        }
        __vic3_native_invoke__?: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>
        __TAURI__?: { core?: { invoke?: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> } }
      }
      const diag = w.__vic3_wdio_diag__
      const meta = diag
        ? {
            nativeInvokeCaptured: diag.nativeInvokeCaptured,
            invokeIsAccessor: diag.invokeIsAccessor,
            mocksKeys: diag.mocksKeys,
            interceptorFlag: diag.interceptorFlag,
            strippedInterception: diag.strippedInterception,
          }
        : undefined
      const out: WdioLoadDiag = {
        meta,
        saveTrace: w.__vic3_desktop_save_trace__,
      }
      if (!diag) return out
      out.listSavesCurrent = await diag.probeCurrent('list_saves')
      out.listSavesNative = await diag.probeNative('list_saves')

      const phase = w.__vic3_desktop_save_trace__?.phase
      // Avoid re-entering use_save if the UI path already reported an error or is mid-flight
      // (phase === 'start' means still awaiting — a second invoke can hang the probe).
      if (phase === 'use_save_ok') {
        out.useSaveCurrent = await diag.probeCurrent('loaded_prices')
        out.useSaveNative = await diag.probeNative('loaded_prices')
        return out
      }
      const shouldProbeUseSave =
        tryUseSave && phase !== 'error' && phase !== 'start' && phase !== 'loaded_prices_ok'
      if (!shouldProbeUseSave) return out

      let location = w.__vic3_desktop_save_trace__?.location
      try {
        const rows = (await (w.__TAURI__?.core?.invoke?.('list_saves') as Promise<
          { name: string; location: string }[]
        >)) ?? []
        location = rows.find((r) => r.name === stubName)?.location ?? location
      } catch {
        /* keep trace location */
      }
      out.resolvedLocation = location
      const args = { name: stubName, location: location ?? 'local' }
      out.useSaveCurrent = await diag.probeCurrent('use_save', args)
      out.useSaveNative = await diag.probeNative('use_save', args)
      return out
    },
    stub,
    opts.tryUseSave,
  )
}

export async function loadTauriSave(saveKey: SaveKey): Promise<void> {
  // Catalog stubs strip the .v3 suffix (normalize_stub).
  const fileName = SAVES[saveKey]
  const stub = fileName.replace(/\.v3$/i, '')
  await $('button*=Saves').click()
  await $('#refresh-saves').click()

  await browser.waitUntil(
    async () => (await $(`td*=${stub}`).isExisting()),
    {
      timeout: 30_000,
      timeoutMsg: `Catalog never listed stub ${stub} (from ${fileName})`,
    },
  )

  const preDiag = await collectTauriLoadDiag(stub, { tryUseSave: false })

  await $(`tr*=${stub}`).$('button*=Load').click()

  // Sample UI immediately — prior red runs never showed chip "Loading…", which
  // distinguishes hang vs fast-fail vs click not reaching React.
  await browser.pause(150)
  const earlyUi = {
    chip: await $('[aria-label="Loaded save"]').getText().catch(() => ''),
    alert: await $('[role="alert"]').getText().catch(() => ''),
    loadBtn: await $(`tr*=${stub}`).$('button').getText().catch(() => ''),
  }

  // use_save switches to Prices before DesktopCatalog can paint "Loaded …" on
  // #saves-status (that node unmounts). Wait for the chip for *this* stub —
  // goods links from a previous save must not satisfy the wait.
  const previousChip = await $('[aria-label="Loaded save"]')
  const previousText = (await previousChip.isExisting()) ? await previousChip.getText() : ''
  try {
    await browser.waitUntil(
      async () => {
        const chip = await $('[aria-label="Loaded save"]')
        if (await chip.isExisting()) {
          const text = await chip.getText()
          if (text.includes(stub)) return true
          // A different save is still displayed — keep waiting.
          return false
        }
        return previousText === '' && (await $$('a.good-link')).length > 0
      },
      {
        timeout: ANALYSIS_TIMEOUT,
        timeoutMsg: `Desktop never showed Loaded save chip / goods after Load ${stub}`,
      },
    )
  } catch (err) {
    const chip = await $('[aria-label="Loaded save"]').getText().catch(() => '')
    const alert = await $('[role="alert"]').getText().catch(() => '')
    const status = await $('#saves-status').getText().catch(() => '')
    const postDiag = await collectTauriLoadDiag(stub, { tryUseSave: true }).catch((probeErr) => ({
      probeError: probeErr instanceof Error ? probeErr.message : String(probeErr),
    }))
    throw new Error(
      `${err instanceof Error ? err.message : String(err)}` +
        ` (chip=${JSON.stringify(chip)}; alert=${JSON.stringify(alert)}; status=${JSON.stringify(status)};` +
        ` earlyUi=${JSON.stringify(earlyUi)}; preDiag=${JSON.stringify(preDiag)};` +
        ` postDiag=${JSON.stringify(postDiag)})`,
    )
  }
  await waitForAnalysisReady()
}

export async function loadSave(saveKey: SaveKey): Promise<void> {
  if (isTauriE2e()) {
    await loadTauriSave(saveKey)
  } else {
    await loadWebSave(saveKey)
  }
}
