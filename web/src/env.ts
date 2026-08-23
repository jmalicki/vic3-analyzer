// Tauri injects window.__TAURI__ into the webview.
// This utility helps us detect if we're running in the Tauri app or a regular browser.

export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI__' in window
}
