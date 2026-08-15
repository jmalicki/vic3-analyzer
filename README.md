# vic3-analyzer

AGPL-3.0 Victoria 3 save loader and planner. Given a `.v3` and a high-level goal (`declare-war`, `research`, `gdp`), it emits a time-optimal move sequence **under our model** — not Paradox’s binary.

**CLI first**, then an in-browser UI (`wasm-bindgen` + React). Saves are never uploaded. Past runs and alternative plans live in a **local archive** (CLI: XDG; UI: IndexedDB).

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
cd web && npm install && npm run dev
```

CI: `cargo fmt`, `clippy -D warnings`, `cargo test --workspace`. Node lint starts in the UI phase.

## License

[AGPL-3.0](LICENSE).
