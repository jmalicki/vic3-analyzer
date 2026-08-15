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
npm run build
```

Unit tests inject a mocked wasm API and use fake IndexedDB. They do not require
the game, wasm-pack, or a real save.

## wasm and definitions

`npm run build:wasm` runs `wasm-pack --target web` into `public/wasm/`
(`vic3_wasm.js` + `.wasm`). Generated wasm artifacts are gitignored.

The app loads `parse_save`, `prices`, `what_if`, `gaps`, `plan`,
`what_if_schema`, and `prices_schema`. Those functions return JSON strings.

`npm run build:defs` writes `public/defs.postcard` from the in-repo
`vic3-defs` fixture tree via `emit_fixture_blob`. The GitHub Pages demo uses
that blob so analysis works without a Victoria 3 install. Do not redistribute
binary token maps.
