# Libraries

P1 contract. Locked items stay locked. Open items are decided in this file before the implementing phase.

**Selection bar:** Prefer crates that keep *our* code simple. Heavier weight (compile time, unused features) is fine when extra complexity is optional and the happy path is not awkward. Reject crates that force a large surface into every call site. Do not pick the smallest crate just to stay “lean.”

Where to look: [crates.io](https://crates.io), [lib.rs](https://lib.rs), [docs.rs](https://docs.rs). Paradox parsing is **jomini**. Numeric equilibrium is **NLS**, not LP.

## Locked

| Piece | Choice |
| --- | --- |
| Clausewitz parse | [jomini](https://crates.io/crates/jomini) (text + binary + envelope). Do not write a parser. |
| Vic3 envelope | pdx-tools `vic3save` git `092edec7` (AGPL) |
| Serde / errors / wasm | serde, thiserror, wasm-bindgen, serde-wasm-bindgen. anyhow only in `vic3-cli` `main`. |
| UI shell | Vite + React |
| CI | GitHub Actions |
| CLI parser | [clap](https://crates.io/crates/clap) in `vic3-cli` only. Not bpaf/argh. Not in wasm. |
| A* | [rust-advanced-heaps](https://github.com/jmalicki/rust-advanced-heaps) git `445818b8` `pathfinding::{SearchNode, shortest_path}`. Not crates.io `pathfinding`. |
| Goal DSL | [chumsky](https://crates.io/crates/chumsky) |
| Property tests | [proptest](https://crates.io/crates/proptest). Not quickcheck. |
| CLI tests | [assert_cmd](https://crates.io/crates/assert_cmd). trycmd deferred. |
| UI tests | Vitest + React Testing Library; wasm mocked. Playwright **deferred** (no dep, no CI job). |
| Price solver | [Basin](https://crates.io/crates/basin) + successive substitution warm start. Not HiGHS/Clarabel/NLopt/IPOPT. |
| Archive | CLI: serde_json files under XDG. UI: IndexedDB. Never uploaded. |
| Desktop config / save catalog | `vic3-catalog`: `dirs`, `toml` (+ `serde_json` for `config.json`). No network. |
| Desktop shell | [Tauri](https://crates.io/crates/tauri) 2 (`vic3-analyzer` binary). Not Electron. |
| SQL engine | [DataFusion](https://crates.io/crates/datafusion) 51 (`vic3-sql`). Not SQLite/GlueSQL. |
| MCP SDK | Official [rmcp](https://crates.io/crates/rmcp) 3.x (`vic3-mcp`, stdio via `transport-io`). |

## Open (decide before the phase)

**Shared option schema (P5):** default that fits the bar is **schemars** on shared serde structs + clap flatten in the CLI. Alternatives (walk clap `Command`, ts-rs/specta, custom `Vic3Options` derive) only if schemars/clap docs drift.

**Basin linear algebra (after P4 if slow):** `Vec` until profiling. nalgebra/faer are optional Basin features; keep BLAS off for wasm.

**IndexedDB wrapper (P6):** `idb` or `idb-keyval` — pick the one whose store API is a handful of functions. Vitest: `fake-indexeddb`.

**GOAP crates:** API inspiration for [`dsl.md`](dsl.md) only, not the planner.
