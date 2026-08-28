# vic3-analyzer web

**Player guide:** [docs/user/web.md](../docs/user/web.md) (defs setup, What-if, Goal gaps / Timeline).

Developer notes below. The prices UI runs locally in the browser. Saves, optional binary token maps,
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

The Vite `base` is `/vic3-analyzer/` for GitHub Pages
(`https://jmalicki.github.io/vic3-analyzer/`). Desktop embeds use
`npm run build:desktop` (`vite build --mode desktop` → `base: '/'`) so Tauri can serve
assets from `tauri://localhost`; the desktop app talks to native Rust via Tauri
`invoke` (no wasm). Web builds keep `base` root-absolute so `loadWasm` can
dynamic-import `/wasm/vic3_wasm.js` — a relative `./wasm/…` would resolve under
`/assets/` and fail with a MIME-type error.

The UI explains token maps / definitions via accessible help and shows the usual
Victoria 3 save folder for your OS. Chromium can remember the last chosen save
folder; other browsers keep the standard file input (sites cannot force an
arbitrary local path).

A deployed build ships no definitions: `npm run build:defs` writes
`fixtures/defs.postcard` outside `public/`, and the fetch for it is behind
`import.meta.env.DEV`, so it exists for development and tests only. Definitions
a user builds or picks are stored in IndexedDB (`defsStore.ts`) and restored on
the next visit, labelled as such, with a button to forget them — otherwise a
reload would quietly leave the app with no definitions at all. The last `.v3`
save (and optional token map) is kept the same way (`saveStore.ts`), together
with the last prices solve so a reload can paint the table before wasm reruns.
Bump `PRICES_CACHE_VERSION` in `saveStore.ts` when the solver or result shape
changes so an old table is not restored.

Analysis tools stay greyed out until definitions are available, and long reads
report progress: a determinate bar over the selected file count and an
indeterminate bar while wasm parses or solves.

For real campaigns, a modal builder creates a versioned `defs.postcard`
entirely in-browser from a dragged-in or locally selected `game` folder — the
only folder the UI asks for. Selecting files does not upload them or use network
bandwidth. The walker prunes heavy directories and reads only supported `common`
definitions, English `goods_l_*.yml`, and the DDS goods icons, which wasm decodes
to PNG data URLs for the tables. Dragging is the reliable route: Chromium's File System Access blocklist marks `~/Library`
(macOS) and `Program Files` (Windows) as block-all-children, and native dialogs
hide those locations, so a dropped folder read through `webkitGetAsEntry` is the
only path-independent option. The card also shows the usual Steam path for your
OS with a copy button and the dialog shortcut (`Cmd+Shift+G`, `Ctrl+L`, or the
Windows address bar). The
files are packed into an in-memory manifest, parsed by `vic3-defs` in wasm, and
never uploaded.

Because a partial pick produces a blob that silently prices only a few goods,
`defs_summary` reports the counts inside whichever blob is active. The builder and
the definitions field both show them, a blob under ten goods is called out as
fixture-sized or incomplete, and swapping the save or definitions clears the
previous result table so it cannot be mistaken for the new inputs' output. The
footer reports the package version, git revision, and UTC build time injected by
Vite.

The goods table sorts by display name, price, and difference from base. Hash
links drill into state-attributed buy/sell orders and then individual building
model IO, costs, revenue, profit, and shortages. State rows use local prices
blended with the solved market price using infrastructure-only access and base
MAPI 75%; wage pops shop at those local prices inside the solver. The global
table shows an order-weighted average for the selected geography. Extra MAPI
modifiers, overseas access, and separate market solves are not yet modeled.
State-attributed orders are access-scaled into the current single market;
building cashflow is not save-native.

## wasm and definitions

`npm run build:wasm` runs `wasm-pack --target web` into `public/wasm/`
(`vic3_wasm.js` + `.wasm`). Generated wasm artifacts are gitignored.

The app loads `parse_save`, `prices`, `what_if`, `gaps`, `plan`,
`what_if_schema`, and `prices_schema`. Those functions return JSON strings.

`npm run build:defs` writes `fixtures/defs.postcard` from the in-repo
`vic3-defs` fixture tree via `emit_fixture_blob`. It backs the wasm wrapper tests
and gives `npm run dev` something to solve; it is not part of `dist/`, so the
GitHub Pages site requires user-supplied definitions.

## Token maps

Victoria 3 writes binary saves by default, so field names are numeric tokens. A
token map is plain text with one `0x1234 field_name` pair per line.

Most players can avoid it: outside Ironman, set
`"save_file_format": "zip_text_all"` in `pdx_settings.json` (in the Victoria 3
documents folder) and re-save. The resulting save loads without a token map. See
[Save-game editing](https://vic3.paradoxwikis.com/Save-game_editing).

Ironman saves stay binary and need a map. There is no official download; Paradox
does not publish the mapping and this project does not redistribute it. Maps are
extracted from a user's own game build, the same user-supplied arrangement other
tools (for example pdx-tools) expect.
