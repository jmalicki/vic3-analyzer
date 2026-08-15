# vic3-analyzer web

The prices UI runs locally in the browser. Saves, optional binary token maps,
and definitions are passed as bytes to `vic3-wasm`; every completed prices or
what-if analysis is stored in IndexedDB. Nothing is uploaded.

## Development

```sh
npm install
npm test
npm run build
```

Unit tests inject a mocked wasm API and use fake IndexedDB. They do not require
the game, wasm-pack, or a real save.

## wasm and definitions

For a browser build, place wasm-pack's web output at
`public/wasm/vic3_wasm.js` and its referenced `.wasm` file beside it. The app
uses the exported `parse_save`, `prices`, `what_if`, `what_if_schema`, and
`prices_schema` functions. Those functions return JSON strings.

Definitions are deliberately not rebuilt from a Victoria 3 installation in
the browser or CI. Supply a prebuilt postcard blob produced offline by
`vic3_defs::encode_blob`. It must match the save's supported game patch.
