# vic3-analyzer web

The prices UI runs locally in the browser. Saves, optional binary token maps,
and definitions are passed as bytes to `vic3-wasm`; every completed prices or
what-if analysis is stored in IndexedDB. Nothing is uploaded.

## Development

```sh
npm install
npm run build:wasm
npm run build:defs
npm test
npm run test:wasm
npm run build
```

`npm test` runs mocked App/RTL tests (no wasm required). `npm run test:wasm`
builds wasm + the fixture defs blob, then exercises the real wrapper against
in-repo save fixtures.

The Vite `base` is `/vic3-analyzer/` so assets resolve on GitHub Pages at
`https://jmalicki.github.io/vic3-analyzer/`. `loadWasm` joins
`import.meta.env.BASE_URL` with `wasm/vic3_wasm.js`.

The UI loads `public/defs.postcard` by default, explains token maps / definitions
via accessible help, and shows the usual Victoria 3 save folder for your OS.
Chromium can remember the last chosen save folder; other browsers keep the
standard file input (sites cannot force an arbitrary local path).

## wasm and definitions

`npm run build:wasm` runs `wasm-pack --target web` into `public/wasm/`
(`vic3_wasm.js` + `.wasm`). Generated wasm artifacts are gitignored.

The app loads `parse_save`, `prices`, `what_if`, `gaps`, `plan`,
`what_if_schema`, and `prices_schema`. Those functions return JSON strings.

`npm run build:defs` writes `public/defs.postcard` from the in-repo
`vic3-defs` fixture tree via `emit_fixture_blob`. The GitHub Pages demo uses
that blob so analysis works without a Victoria 3 install. Do not redistribute
binary token maps.
