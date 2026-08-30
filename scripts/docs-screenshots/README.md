# Docs screenshots

Playwright harness that regenerates locked PNGs under [`docs/assets/`](../../docs/assets/README.md).

## Prerequisites

- Node 22+
- Rust toolchain + `wasm-pack` (same as the web app)
- Chromium via Playwright (`npx playwright install chromium`)

## Install

```bash
pnpm install
pnpm --filter vic3-docs-screenshots exec playwright install chromium
```

Also install and build the web app once (wasm + fixture defs + `dist/`):

```bash
pnpm install
pnpm --filter web run build
```

## Regenerate locally

From repo root or `scripts/docs-screenshots/`:

```bash
pnpm --filter vic3-docs-screenshots run docs:screenshots          # web + desktop mock → docs/assets/
pnpm --filter vic3-docs-screenshots run docs:screenshots:web
pnpm --filter vic3-docs-screenshots run docs:screenshots:desktop:mock
```

Override output directory (CI does this):

```bash
DOCS_SCREENSHOTS_OUT=/tmp/docs-shots pnpm --filter vic3-docs-screenshots run docs:screenshots
DOCS_SCREENSHOTS_OUT=/tmp/docs-shots pnpm --filter vic3-docs-screenshots run docs:screenshots:compare
```

### Compare policy

`docs:screenshots:compare` pixel-diffs generated files against committed `docs/assets/*.png`.

- **No golden yet** → skip that filename (exit 0). This is how PR2 CI stays green before assets land.
- **Golden present** → fail on drift (small threshold; diffs written under `$OUT/_diff/`).

## What each generator does

| Script | How |
| --- | --- |
| **web** | `vite preview` of `web/dist`, seeds fixture defs into IndexedDB, uploads the plaintext save fixture, walks hash/nav routes, writes `web-*.png` |
| **desktop:mock** | Serves `web/dist` with `window.__TAURI__.core.invoke` mocked from `fixtures/desktop-mock-data.json` — **CI layout drift** only (Query pane needs the Tauri mock) |
| **desktop:tauri** | Real companion on **macOS** via `@wdio/tauri-service` + `screencapture -l` (native window chrome). Prefer this when committing `desktop-*.png` |

Viewport (web + mock): **1280×800 @ 2×** `deviceScaleFactor`. Tauri shots use the live window size from `tauri.conf.json` (1200×820).

## Real macOS Tauri (committed desktop goldens)

```bash
pnpm --filter vic3-docs-screenshots run docs:screenshots:desktop:tauri
```

Requires Screen Recording permission for your terminal. Details: [`desktop-tauri/README.md`](desktop-tauri/README.md). Ubuntu CI keeps the mock path only.

## CI

Job `docs-screenshots` on `ubuntu-latest` builds the web app, runs web + desktop mock into a scratch dir, then compares only when goldens exist. Artifacts upload on failure.
