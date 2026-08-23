# Contributing to Victoria 3 Analyzer

Thank you for contributing to **Victoria 3 Analyzer**! This guide covers repository setup, local development workflows, testing, and technical documentation.

---

## 🛠️ Prerequisites

* **Rust:** Stable toolchain (2021 edition).
* **Node.js:** v18+ and `npm` (for the Web UI).
* **Git:** With pre-commit hooks configured.

---

## 🚀 Quickstart & Development Workflow

### 1. Configure Git Hooks
We use pre-commit hooks to ensure `rustfmt`, `clippy`, and TypeScript checks pass before each commit:

```bash
git config core.hooksPath .githooks
```

### 2. Run Workspace Tests & Linters
Always ensure all tests and linter checks pass before opening a pull request:

```bash
# Run all Rust workspace unit and property tests
cargo test --workspace

# Run the complete formatting and linting suite (Rust + TypeScript)
./scripts/lint.sh

# Run headless MCP smoke test
./scripts/mcp-smoke.sh
```

### 3. Run the Web App Locally
The browser frontend runs with Vite and communicates with `vic3-wasm`:

```bash
cd web
npm install
npm run build:wasm     # Compiles crates/vic3-wasm to web/public/wasm
npm run build:defs     # Generates test definitions if needed
npm run dev            # Starts Vite local dev server at http://localhost:5173
```

### 4. Run the Native Desktop Companion
Build the web UI into `web/dist` with a Tauri-relative asset base, then run the
binary (debug loads the embedded dist — no Vite server required):

```bash
npm run build:desktop --prefix web
cargo run -p vic3-analyzer
```

Rebuild `web` after frontend changes; Rust re-embeds `frontendDist` on the next
`cargo run`. For browser-only work, use §3 (`npm run dev`) instead.

---

## 📚 Technical Documentation Directory

Deep technical specifications and architectural contracts are organized under `docs/`:

| Document | Area | Description |
| :--- | :--- | :--- |
| **[Architecture Specification](docs/architecture.md)** | Architecture | Crate boundaries, dataflow pipeline, asset decoder, and WASM bridge. |
| **[Mathematical Invariants](docs/invariants.md)** | Correctness | Formal property-tested invariants (I1–I9) and solver guarantees. |
| **[Dependency Registry](docs/libraries.md)** | Dependencies | Crate selection criteria, locked libraries, and WASM constraints. |
| **[Model Context Protocol (MCP)](docs/mcp.md)** | Integrations | AI assistant tool schemas, resources, and JSON-RPC contracts. |
| **[DataFusion SQL Interface](docs/sql.md)** | Integrations | Embedded read-only SQL fact tables (`active.*`) and analytical TVFs. |
| **[CLI Reference](docs/cli.md)** | Tooling | Subcommands, headless batch automation, and save patching. |
| **[JSON Schema Contracts](docs/json-schema.md)** | Schemas | Shared option and analytical result schema contracts (draft 2020-12). |
| **[Price Equilibrium Methodology](docs/prices.md)** | Economic Engine | Non-linear market solver, MAPI formulas, and pop demand. |
| **[Strategic Planning Engine](docs/planning.md)** | Planning Engine | State projection, A* graph search, and heuristic bounds. |

---

## 🔒 Security & Asset Policy

* **No Proprietary Assets:** Never commit extracted game data blobs (`defs.postcard`), game textures (`.dds`), or token mapping files derived from Paradox game files.
* **Privacy First:** The analysis engine is strictly offline. No save data, tokens, or campaign analytics are ever transmitted over the network.
