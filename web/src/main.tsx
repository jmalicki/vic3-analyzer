import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'

// Guest WDIO harness for Tauri e2e / docs screenshots (`vite build --mode desktop`).
// Sets window.__wdio_original_core__ / window.wdioTauri for @wdio/tauri-service.
if (import.meta.env.MODE === 'desktop' || import.meta.env.VITE_TAURI === '1') {
  await import('@wdio/tauri-plugin')
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
