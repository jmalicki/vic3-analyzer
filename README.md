# vic3-analyzer

AGPL-3.0 Victoria 3 save loader and planner. Given a `.v3` and a high-level goal (`declare-war`, `research`, `gdp`), it emits a time-optimal move sequence **under our model** — not Paradox’s binary.

**CLI first**, then an in-browser UI (`wasm-bindgen` + React). Saves are never uploaded. Past runs and alternative plans live in a **local archive** (CLI: XDG; UI: IndexedDB).

**Live demo:** [https://jmalicki.github.io/vic3-analyzer/](https://jmalicki.github.io/vic3-analyzer/) — drop a plaintext `.v3` (or fixture bytes) and supply definitions from your own install. The site ships none, so nothing can quietly price a campaign from test fixtures. Binary token maps are not redistributed either.

Build a blob from your installed game:

```text
cargo run -p vic3-cli -- defs export \
  --game "/path/to/Victoria 3/game" \
  --out defs.postcard
```

Upload `defs.postcard` in the web UI. Do not commit or publish it: it is derived
from Paradox game data.

Alternatively, the web UI can build the v2 blob locally: drag the installed
`Victoria 3/game` folder onto the builder (or use `game/common` without localized
names), pick it, or supply a zip. The SPA prunes heavy trees and reads only
allowlisted definitions plus English goods localization. Dragging is the
reliable route, since Chrome refuses to open Steam's install
location (`~/Library`, `Program Files`) through its folder pickers. The
Clausewitz files are parsed in wasm and never leave the browser, and the built
blob is kept in IndexedDB so a reload does not lose it.

Goods results are sortable and link to state-attributed orders, then to
individual building model revenue, costs, profit, IO, and shortages. Prices are
currently one shared whole-save synthetic market price; state-local MAPI is not
yet modeled.

## Docs

| Doc | Contents |
| --- | --- |
| [architecture](docs/architecture.md) | Crates, data flow, what is not the game |
| [usage](docs/usage.md) | CLI/UI, shared option structs |
| [dsl](docs/dsl.md) | Goal language (chumsky) |
| [prices](docs/prices.md) | Equilibrium, Basin, limitations |
| [planning](docs/planning.md) | `PlanningState`, A* |
| [archive](docs/archive.md) | Past saves and alternative plans |
| [libraries](docs/libraries.md) | Locked deps and selection bar |
| [invariants](docs/invariants.md) | I1–I9 property tests |
| [json-schema](docs/json-schema.md) | Result/option contract (draft) |

## Develop

```text
cargo test --workspace
cargo run -p vic3-cli
cd web && npm install && npm run build:wasm && npm run build:defs && npm run dev
```

CI runs Rust (`fmt`, `clippy`, `test`) and the web job (`npm test`, `npm run test:wasm`, `npm run build`). Pushes to `main` deploy `web/dist` to GitHub Pages.

## License

[AGPL-3.0](LICENSE).
