# Real macOS Tauri screenshots

Produces the locked `desktop-*.png` files under [`docs/assets/`](../../../docs/assets/README.md) from a **real** companion window (native macOS chrome included).

Ubuntu CI keeps using [`capture-desktop-mock.mjs`](../capture-desktop-mock.mjs) for layout drift. **Committed desktop goldens should come from this path on a Mac.**

## Requirements

- macOS (Screen Recording permission for Terminal/Cursor so `screencapture -l` works)
- Rust + Node 22+
- One-time: `npm ci` in `scripts/docs-screenshots` and in this folder

## Run

From `scripts/docs-screenshots/`:

```bash
npm run docs:screenshots:desktop:tauri
```

That script:

1. Builds `vic3-analyzer` with `--features webdriver` and a temporary WebDriver capability
2. Seeds a disposable `XDG_DATA_HOME` with fixture save + defs
3. Launches the app via `@wdio/tauri-service` (embedded WebDriver)
4. Walks Dashboard → Saves → **Alerts** → Query → States → Prices → Timeline → Settings
5. Captures each step with macOS `screencapture -l <windowID>` so the title bar / traffic lights are in the PNG

## Notes

- `tauri-plugin-wdio-webdriver` and `tauri-plugin-wdio` (plus frontend `@wdio/tauri-plugin`) are optional (`webdriver` feature / desktop embed) and only initialized for these builds — do not ship that feature to players.
- If capture fails with a blank/permission error, enable **Screen Recording** for your terminal in System Settings → Privacy & Security.
