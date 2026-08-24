import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'

/**
 * Embedded `@wdio/tauri-service` focus recovery calls
 * `plugin:wdio|get_window_states` via direct eval, which waits on
 * `window.__wdio_original_core__`. The full `@wdio/tauri-plugin` also
 * Proxies `core.invoke` for mocking — that wrapper breaks companion
 * `use_save` / IPC on WebKit in CI. Snapshot the real core only.
 */
function installWdioCoreBridge(): void {
  if (!(import.meta.env.MODE === 'desktop' || import.meta.env.VITE_TAURI === '1')) {
    return
  }
  const w = window as Window & {
    __TAURI__?: { core?: { invoke?: unknown } }
    __wdio_original_core__?: unknown
  }
  const trySnap = () => {
    const core = w.__TAURI__?.core
    if (core && typeof core.invoke === 'function') {
      w.__wdio_original_core__ = core
      return
    }
    window.setTimeout(trySnap, 50)
  }
  trySnap()
}

installWdioCoreBridge()

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
