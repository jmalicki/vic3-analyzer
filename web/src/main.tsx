import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'

// Match Tauri's InvokeArgs loosely — plugin typings use a wider union than Record.
type TauriInvoke = (cmd: string, args?: unknown) => Promise<unknown>

type Vic3WdioDiag = {
  /** True once we captured a real invoke before @wdio/tauri-plugin wrapped it. */
  nativeInvokeCaptured: boolean
  /** True when core.invoke is an accessor (plugin defineProperty path). */
  invokeIsAccessor: boolean
  mocksKeys: string[]
  interceptorFlag: boolean
  strippedInterception: boolean
  /**
   * Call through whatever `window.__TAURI__.core.invoke` is *right now*
   * (wrapped or not).
   */
  probeCurrent: (cmd: string, args?: Record<string, unknown>) => Promise<ProbeResult>
  /** Call the pre-wrap native function if we still have it. */
  probeNative: (cmd: string, args?: Record<string, unknown>) => Promise<ProbeResult>
}

type ProbeResult = {
  ok: boolean
  ms: number
  resultType?: string
  error?: string
}

type Vic3DesktopSaveTrace = {
  phase: 'start' | 'use_save_ok' | 'loaded_prices_ok' | 'error'
  stub?: string
  location?: string
  at: number
  error?: string
  invokePath?: 'current' | 'missing'
}

declare global {
  interface Window {
    __vic3_native_invoke__?: TauriInvoke
    __vic3_wdio_diag__?: Vic3WdioDiag
    __vic3_desktop_save_trace__?: Vic3DesktopSaveTrace
  }
}

function isDesktopEmbed(): boolean {
  return import.meta.env.MODE === 'desktop' || import.meta.env.VITE_TAURI === '1'
}

async function waitForCoreInvoke(timeoutMs = 10_000): Promise<TauriInvoke | undefined> {
  const start = Date.now()
  while (Date.now() - start < timeoutMs) {
    const invoke = window.__TAURI__?.core?.invoke
    if (typeof invoke === 'function') {
      return invoke.bind(window.__TAURI__!.core!) as TauriInvoke
    }
    await new Promise((r) => window.setTimeout(r, 25))
  }
  return undefined
}

function describeInvokeAccessor(): boolean {
  const core = window.__TAURI__?.core
  if (!core) return false
  const desc = Object.getOwnPropertyDescriptor(core, 'invoke')
  return Boolean(desc && ('get' in desc || 'set' in desc))
}

async function runProbe(
  invoke: TauriInvoke | undefined,
  cmd: string,
  args?: Record<string, unknown>,
): Promise<ProbeResult> {
  if (!invoke) {
    return { ok: false, ms: 0, error: 'invoke function missing' }
  }
  const t0 = performance.now()
  try {
    const result = await invoke(cmd, args)
    return {
      ok: true,
      ms: Math.round(performance.now() - t0),
      resultType: result === null || result === undefined ? String(result) : typeof result,
    }
  } catch (reason) {
    return {
      ok: false,
      ms: Math.round(performance.now() - t0),
      error: reason instanceof Error ? reason.message : String(reason),
    }
  }
}

function installDiag(native: TauriInvoke | undefined, stripped: boolean): void {
  const refreshMeta = () => {
    const core = window.__TAURI__?.core as { _wdioInvokeInterceptor?: boolean } | undefined
    return {
      nativeInvokeCaptured: typeof native === 'function',
      invokeIsAccessor: describeInvokeAccessor(),
      mocksKeys: Object.keys(window.__wdio_mocks__ ?? {}),
      interceptorFlag: Boolean(core?._wdioInvokeInterceptor),
      strippedInterception: stripped,
    }
  }

  window.__vic3_wdio_diag__ = {
    ...refreshMeta(),
    probeCurrent: async (cmd, args) => {
      Object.assign(window.__vic3_wdio_diag__!, refreshMeta())
      const invoke = window.__TAURI__?.core?.invoke as TauriInvoke | undefined
      return runProbe(typeof invoke === 'function' ? invoke.bind(window.__TAURI__!.core!) : undefined, cmd, args)
    },
    probeNative: async (cmd, args) => {
      Object.assign(window.__vic3_wdio_diag__!, refreshMeta())
      return runProbe(window.__vic3_native_invoke__ ?? native, cmd, args)
    },
  }
}

/**
 * Guest WDIO harness for Tauri e2e / docs screenshots (`vite build --mode desktop`).
 *
 * Loads the official `@wdio/tauri-plugin` (focus recovery + optional mocks). We
 * snapshot the *function* `core.invoke` *before* the plugin's defineProperty
 * wrapper so e2e can A/B current vs native IPC. Set
 * `VITE_WDIO_STRIP_INVOKE_INTERCEPT=1` to restore the pre-wrap invoke after
 * init (controlled bisect — not the default).
 */
async function bootWdioGuest(): Promise<void> {
  if (!isDesktopEmbed()) return

  const native = await waitForCoreInvoke()
  if (native) {
    window.__vic3_native_invoke__ = native
    // Prefer a stable original for @wdio/tauri-service focus recovery even if
    // the plugin later points __wdio_original_core__ at the same mutated object.
    window.__wdio_original_core__ = { invoke: native }
  }

  await import('@wdio/tauri-plugin')
  await window.wdioTauri?.waitForInit?.()

  const strip = import.meta.env.VITE_WDIO_STRIP_INVOKE_INTERCEPT === '1'
  if (strip && native && window.__TAURI__?.core) {
    Object.defineProperty(window.__TAURI__.core, 'invoke', {
      value: native,
      writable: true,
      configurable: true,
      enumerable: true,
    })
    const core = window.__TAURI__.core as { _wdioInvokeInterceptor?: boolean }
    delete core._wdioInvokeInterceptor
    window.__wdio_original_core__ = { invoke: native }
  }

  installDiag(native, strip)
}

await bootWdioGuest()

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
