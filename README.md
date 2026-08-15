# vic3-analyzer

AGPL-3.0 Victoria 3 save loader and planner. Given a `.v3` and a high-level goal (`declare-war`, `research`, `gdp`), it emits a time-optimal move sequence **under our model** — not Paradox’s binary.

**CLI first**, then an in-browser UI (`wasm-bindgen` + React). Saves are never uploaded. Past runs and alternative plans live in a **local archive** (CLI: XDG; UI: IndexedDB).

**Live demo:** [https://jmalicki.github.io/vic3-analyzer/](https://jmalicki.github.io/vic3-analyzer/) — drop a plaintext `.v3` (or fixture bytes); the site ships a fixture defs postcard so prices/what-if/plan work without a game install. Binary token maps are not redistributed.

The hosted definitions are intentionally tiny test fixtures. For all goods in
a real campaign, build a private blob from your installed game:

```text
cargo run -p vic3-cli -- defs export \
  --game "/path/to/Victoria 3/game" \
  --out defs.postcard
```

Upload `defs.postcard` in the web UI. Do not commit or publish it: it is derived
from Paradox game data.

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
