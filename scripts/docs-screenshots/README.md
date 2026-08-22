# Docs screenshots

Playwright harness that regenerates locked PNGs under [`docs/assets/`](../../docs/assets/README.md).

## Prerequisites

- Node 22+
- Rust toolchain + `wasm-pack` (same as the web app)
- Chromium via Playwright (`npx playwright install chromium`)

## Install

```bash
cd scripts/docs-screenshots
npm ci
npx playwright install chromium
```

Also install and build the web app once (wasm + fixture defs + `dist/`):

```bash
cd web
npm ci
npm run build
```

## Regenerate locally

From `scripts/docs-screenshots/`:

```bash
npm run docs:screenshots          # web + desktop mock → docs/assets/
npm run docs:screenshots:web
npm run docs:screenshots:desktop:mock
```

Override output directory (CI does this):

```bash
DOCS_SCREENSHOTS_OUT=/tmp/docs-shots npm run docs:screenshots
DOCS_SCREENSHOTS_OUT=/tmp/docs-shots npm run docs:screenshots:compare
```

### Compare policy

`docs:screenshots:compare` pixel-diffs generated files against committed `docs/assets/*.png`.

- **No golden yet** → skip that filename (exit 0). This is how PR2 CI stays green before assets land.
- **Golden present** → fail on drift (small threshold; diffs written under `$OUT/_diff/`).

## What each generator does

| Script | How |
| --- | --- |
| **web** | `vite preview` of `web/dist`, seeds fixture defs into IndexedDB, uploads the plaintext save fixture, walks hash/nav routes, writes `web-*.png` |
| **desktop:mock** | Serves `crates/vic3-analyzer/ui/index.html` with `window.__TAURI__.core.invoke` mocked from `fixtures/desktop-mock-data.json`, writes `desktop-*.png` |
| **desktop:tauri** | Stub — real macOS Tauri via WDIO (see below) |

Viewport: **1280×800 @ 2×** `deviceScaleFactor`.

## Real macOS Tauri (optional / future)

Vanilla Playwright cannot drive WKWebView. Prefer Tauri’s WebDriver stack on macOS when you need the native chrome:

- [WebDriver / `@wdio/tauri-service`](https://v2.tauri.app/develop/tests/webdriver/) + debug-only `tauri-plugin-wdio-webdriver`
- Or a Playwright bridge such as [`@srsholmes/tauri-playwright`](https://github.com/srsholmes/tauri-playwright)

See [`desktop-tauri/README.md`](desktop-tauri/README.md). Not required for Ubuntu CI; CI uses the mock path only.

## CI

Job `docs-screenshots` on `ubuntu-latest` builds the web app, runs web + desktop mock into a scratch dir, then compares only when goldens exist. Artifacts upload on failure.
