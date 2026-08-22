# Dependency Selection & Crate Registry

This document records the foundational library choices, dependency selection criteria, and architectural constraints across the `vic3-analyzer` workspace.

---

## Selection Criteria

1. **Simplicity and Maintainability:** Favor libraries that simplify call sites and minimize boilerplate over hyper-minimalist crates that require custom wrappers.
2. **WASM & Native Portability:** Core analysis libraries must compile to `wasm32-unknown-unknown` without C dependencies, dynamic thread pools, or native BLAS linkages.
3. **Reproducibility & Determinism:** Parsers and solver libraries must produce deterministic results across platforms.

---

## Crate Selection Registry

| Domain / Responsibility | Selected Crate | Rationale & Alternatives Considered |
| --- | --- | --- |
| **Clausewitz Parsing** | [`jomini`](https://crates.io/crates/jomini) | Industry standard for Clausewitz text/binary saves and token mapping. |
| **Save Envelope** | `pdx-tools` (`vic3save`) | Specialized parser for Victoria 3 zip/binary envelopes. |
| **Serialization** | `serde`, `serde_json`, `postcard` | Fast, reliable serialization. `postcard` provides compact binary blobs for definitions in WASM. |
| **WASM Bridge** | `wasm-bindgen`, `serde-wasm-bindgen` | Official Rust-WASM toolchain. |
| **UI Shell** | Vite + React | Modern, fast web frontend ecosystem with IndexedDB local persistence. |
| **Desktop Shell** | [Tauri 2](https://v2.tauri.app/) (`crates/vic3-analyzer`) | Lightweight native desktop shell with low memory footprint compared to Electron. |
| **CLI Parser** | [`clap`](https://crates.io/crates/clap) | Comprehensive CLI argument parsing (isolated exclusively to `vic3-cli`). |
| **A* Pathfinding** | [`rust-advanced-heaps`](https://github.com/jmalicki/rust-advanced-heaps) | High-performance priority queue heaps (`PairingHeap`) and shortest-path graph search. |
| **Goal DSL Parser** | [`chumsky`](https://crates.io/crates/chumsky) | Expressive parser combinator library with excellent error reporting. |
| **Property Testing** | [`proptest`](https://crates.io/crates/proptest) | Robust hypothesis-style property testing for mathematical invariants. |
| **Non-Linear Solver** | [`basin`](https://crates.io/crates/basin) | Trust-region non-linear least squares solver with pure-Rust vector math (no BLAS needed in WASM). |
| **Embedded SQL** | [Apache DataFusion](https://datafusion.apache.org/) (`vic3-sql`) | Embedded columnar SQL query engine over in-memory Arrow record batches. |
| **MCP SDK** | [`rmcp`](https://crates.io/crates/rmcp) | Official Rust Model Context Protocol implementation for stdio AI agent communication. |
| **Schema Generation** | [`schemars`](https://crates.io/crates/schemars) | Generates JSON Schema draft 2020-12 from shared serde structs to drive UI forms. |
