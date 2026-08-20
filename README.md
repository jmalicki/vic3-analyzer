# vic3-analyzer

AGPL-3.0 Victoria 3 save loader and planner. Given a `.v3` and a high-level goal, it evaluates readiness gaps and — when the simulator has actions for that goal — emits a time-optimal move sequence **under our model**, not Paradox’s binary. Today A* closes `research`, modeled `gdp`, supported goods-price goals, `interest_in`, `army_power_projection`, and solvency / credit-headroom via a compact payday model; full `declare-war` works when munitions and solvent already hold (solvent can also become true via payday), while weekly-income and SoL presets stay gaps diagnostics until their actions exist (see [`docs/dsl.md`](docs/dsl.md)).

**CLI first**, then an in-browser UI (`wasm-bindgen` + React). Saves are never uploaded. Past runs and alternative plans live in a **local archive** (CLI: XDG; UI: IndexedDB).

**Live demo:** [https://jmalicki.github.io/vic3-analyzer/](https://jmalicki.github.io/vic3-analyzer/) — drop a plaintext `.v3` (or fixture bytes) and supply definitions from your own install. The site ships none, so nothing can quietly price a campaign from test fixtures. Binary token maps are not redistributed either.

The browser builds and stores definitions locally when you choose or drag your
installed `Victoria 3/game` folder. It prunes heavy trees and reads only
allowlisted definitions, English goods localization, and the goods icons under
`gfx/interface/icons/goods_icons`. Selecting a local folder does not upload it
or use network bandwidth.

Icons ship as DDS, which browsers cannot draw, so the loader decodes the top mip
(BC1/BC2/BC3/BC7 or uncompressed 32-bit) and stores PNG bytes in the blob.
Anything it cannot decode is skipped: goods then render by name alone.

For CLI use, build a blob directly from your installed game:

```text
cargo run -p vic3-cli -- defs export \
  --game "/path/to/Victoria 3/game" \
  --out defs.postcard
```

Do not commit or publish `defs.postcard`: it is derived from Paradox game data.

Dragging the game folder is the reliable browser route, since Chrome refuses to open Steam's install
location (`~/Library`, `Program Files`) through its folder pickers. The
Clausewitz files are parsed in wasm and never leave the browser, and the built
blob is kept in IndexedDB so a reload does not lose it.

Goods results are sortable, show the game's icon and localized name, and link to
state-attributed orders, then to
individual building model revenue, costs, profit, IO, and shortages. Prices are
per-state MAPI blends of the solved market price and attributed local orders,
using infrastructure-only access and base MAPI 75%; the global table shows an
order-weighted average for the selected geography. Full modifiers, overseas
access, and separate market solves are not yet modeled. State-attributed orders
are access-scaled into the current single market, including post-1.9 trade
recorded directly on each trade-center state.

## Docs

| Doc | Contents |
| --- | --- |
| [architecture](docs/architecture.md) | Crates, data flow, what is not the game |
| [usage](docs/usage.md) | CLI/UI, shared option structs |
| [dsl](docs/dsl.md) | Goal language (chumsky) |
| [prices](docs/prices.md) | Equilibrium, Basin, limitations |
| [planning](docs/planning.md) | `PlanningState`, A* |
| [archive](docs/archive.md) | Past saves and alternative plans |
| [sql](docs/sql.md) | **Design (review):** DataFusion SQL tables / UDFs / `plan()` TVF |
| [mcp](docs/mcp.md) | **Design (review):** stdio MCP tools / resources / agent flow |
| [desktop](docs/desktop.md) | Desktop auto-detect, `vic3-catalog` config, Settings |
| [desktop](docs/desktop.md) | **Design (review):** Tauri auto-detect, config, Settings |
| [desktop](docs/desktop.md) | Tauri shell build/run + **design (review):** auto-detect, config, Settings |
| [libraries](docs/libraries.md) | Locked deps and selection bar |
| [invariants](docs/invariants.md) | I1–I9 property tests |
| [json-schema](docs/json-schema.md) | Result/option contract (draft) |

## Develop

```text
git config core.hooksPath .githooks   # fmt, clippy, web tsc on commit (same as CI lint)
cargo test --workspace
cargo run -p vic3-cli
cd web && npm install && npm run build:wasm && npm run build:defs && npm run dev
```

CI runs Rust (`fmt`, `clippy`, `test`) and the web job (`npm test`, `npm run test:wasm`, `npm run build`). Pushes to `main` deploy `web/dist` to GitHub Pages. Pre-commit is `./scripts/lint.sh` (fmt, clippy, `tsc -b`); it does not run tests or wasm-pack.

## License

[AGPL-3.0](LICENSE).
