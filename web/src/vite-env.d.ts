/// <reference types="vite/client" />

declare const __APP_VERSION__: string
declare const __BUILD_TIME__: string
declare const __GIT_REVISION__: string

interface ImportMetaEnv {
  readonly VITE_TAURI?: string
  readonly VITE_WDIO_STRIP_INVOKE_INTERCEPT?: string
}

interface Window {
  __vic3_desktop_save_trace__?: {
    phase: 'start' | 'use_save_ok' | 'loaded_prices_ok' | 'error'
    stub?: string
    location?: string
    at: number
    error?: string
    invokePath?: 'current' | 'missing'
  }
}
