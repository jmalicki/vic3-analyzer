# vic3-analyzer

AGPL-3.0 Victoria 3 save loader and planner. Given a `.v3` and a high-level goal (`declare-war`, `research`, `gdp`), it emits a time-optimal move sequence **under our model** — not Paradox’s binary.

**CLI first**, then an in-browser UI (`wasm-bindgen` + React). Saves are never uploaded. Past runs and alternative plans live in a **local archive** (CLI: XDG data dir; UI: IndexedDB).

Design contract: [`docs/`](docs/). Start with [`docs/architecture.md`](docs/architecture.md) and [`docs/usage.md`](docs/usage.md).

## Status

Scaffolding. See the phase plan in-repo once `docs/` lands.

## License

[AGPL-3.0](LICENSE).
