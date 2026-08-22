# vic3-analyzer

> Privacy-first, offline Victoria 3 save analyzer, economic solver, and strategic planner.

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](LICENSE)
[![CI](https://github.com/jmalicki/vic3-analyzer/actions/workflows/ci.yml/badge.svg)](https://github.com/jmalicki/vic3-analyzer/actions)

**vic3-analyzer** is an offline second-screen companion for *Victoria 3* (`.v3` save files). It solves true market price equilibria, diagnoses regional economic bottlenecks, checks strategic readiness, and finds optimal action timelines to achieve national goals.

---

## 🎮 How This Enhances Your Victoria 3 Campaign

Victoria 3 gives you immense freedom, but managing a 19th-century industrial empire comes with complex feedback loops and decision paralysis. `vic3-analyzer` acts as your personal chief economic advisor and strategic staff:

- **Avoid the Construction Trap (No More Blind Queueing):** In-game, queuing 10 Steel Mills can quietly spike input iron/coal prices to +75% and crash steel prices to -75% two years later, rendering the factories unprofitable. **What-If Previews** let you simulate the exact market and state-level price equilibrium before spending construction points.
- **Solve Qualification Deadlocks:** Wondering why your chemical plant won't hire despite high unemployment? The tool audits local pop literacy, wealth, and existing professions to give actionable promotion advice (e.g. *"Build a University here: you have high-literacy Machinists ready to promote to Engineers; building farms won't help"*).
- **Never Botch War Mobilization:** Before launching a diplomatic play for Alsace-Lorraine or the Congo, check your **Readiness Gaps**. The planner verifies strategic interest, troop numbers, munitions price stability, and debt runway under war mobilization so you don't default mid-war.
- **Optimize Production Methods (PMs):** When researching new techs (like Atmospheric Engines or Fertilizer), test which PM combinations maximize revenue, productivity, or Standard of Living across your states without triggering severe domestic shortages.
- **AI Strategic Co-Pilot (via MCP):** Connect an LLM (such as Claude Desktop or Cursor) to your campaign as an autonomous Chief of Staff. You can chat in natural language (*"Why is my weekly balance dropping?", "Am I ready to declare war on Austria?", "Simulate switching to Bessemer steel"*), and the AI executes real-time queries and what-if solves directly against your save.
- **Zero Friction & 100% Offline Privacy:** Keep it open in a browser tab or native desktop window. Your save files, token maps, and campaigns are parsed locally and **never uploaded**.

---

## 🔬 Why We Solve Market Equilibria

Victoria 3's economy cannot be accurately analyzed by simply tallying current buy and sell orders. When production changes or buildings expand:
- **Pop demand is dynamic:** Pops shop across tiered need packages; as prices move, pop substitution and purchasing power shift requested quantities.
- **Local prices depend on market access (MAPI):** State-level prices blend local orders with national market prices based on infrastructure access.
- **Cascading profitability:** Factory revenue and input costs change simultaneously across the entire supply chain.

Rather than relying on un-equilibrated estimates, `vic3-analyzer` solves the full non-linear system (NLS) to find consistent pop demand and market prices. This gives accurate revenue, profit, and shortage projections for what-if scenarios and goal planning. *(See [Price Methodology](docs/prices.md) for mathematical formulas and solver mechanics).*

---

## 🌐 Four Ways to Run

### 1. In-Browser Web App (Zero Install)
* **Live Demo:** [https://jmalicki.github.io/vic3-analyzer/](https://jmalicki.github.io/vic3-analyzer/)
* Drag and drop your `.v3` save and your local `Victoria 3/game` folder.
* **100% Client-Side:** Everything parses and solves locally inside your browser via WebAssembly (`wasm-bindgen`) and IndexedDB. Saves, definitions, and tokens are **never uploaded** to any server.

### 2. AI Strategic Co-Pilot (Claude Desktop / Cursor via MCP)
* Turn Claude Desktop, Cursor, or your favorite AI assistant into your personal Grand Strategy advisor using the [Model Context Protocol (MCP)](docs/mcp.md).
* Connects seamlessly to your desktop AI tools via standard local process communication (no web servers or open network ports required).
* The AI can automatically discover your autosaves, explain domestic shortages, run SQL queries, and preview what-if economic adjustments in real time:
  ```json
  {
    "mcpServers": {
      "vic3-analyzer": {
        "command": "cargo",
        "args": ["run", "-p", "vic3-analyzer", "--", "mcp"]
      }
    }
  }
  ```

### 3. Native Desktop Companion (Tauri 2 GUI)
* Standalone desktop application with auto-discovery of local Steam installs and save directories across macOS, Windows, and Linux.
* Includes an interactive campaign dashboard, save timeline browser, and what-if simulation tools.
* Run locally:
  ```bash
  cargo run -p vic3-analyzer
  ```

### 4. Command Line Interface (CLI)
* Fast batch price solves, qualification alerts, what-if deltas, and save patching.
  ```bash
  cargo run -p vic3-cli -- prices --save campaign.v3 --game "/path/to/Victoria 3/game"
  ```

---

## ✨ Core Capabilities

- **Market Equilibrium & MAPI Pricing:** Computes relative market prices and per-state blended prices considering pop wealth, substitution shares, and infrastructure access.
- **What-If Economic Simulator:** Test building expansions and production method (PM) switches with instant re-solved price previews before committing in-game.
- **Strategic Goal Planning:** Evaluate readiness gaps and generate step-by-step action sequences for goals such as `declare-war(state=alsace)`, `gdp >= 100M`, `research(tech=nitroglycerin)`, and credit solvency.
- **Bottleneck Diagnostics & Qualification Advice:** State-by-state pop employment shortage detection with contextual promotion feeder advice (e.g. promoting high-literacy machinists to engineers via universities).
- **Privacy & Asset Safety:** All game parsing is local. No proprietary Paradox game files, textures, or binary token maps are redistributed.

---

## 📚 Documentation Directory

### User & Player Guides
| Document | Description |
| --- | --- |
| [Usage Guide](docs/usage.md) | CLI commands, web UI navigation, shared options, and definitions workflow |
| [Desktop Companion](docs/desktop.md) | Tauri app setup, automatic Steam/save discovery, and settings |
| [Goal DSL](docs/dsl.md) | Goal language grammar, predicates, and expansion rules (`declare-war`, `colonize`) |
| [Archive & History](docs/archive.md) | Local save archive, timeline branching, and historical plan comparison |

### Economic & Planning Methodology
| Document | Description |
| --- | --- |
| [Price Equilibrium](docs/prices.md) | Non-linear solver, MAPI formulas, pop need packages, and solver limitations |
| [Strategic Planning](docs/planning.md) | `PlanningState` projection, A* graph search, heuristic bounds, and payday debt model |

### Integrations & Developer Tools
| Document | Description |
| --- | --- |
| [MCP Server](docs/mcp.md) | Connect AI assistants (Claude Desktop, Cursor) to your campaign via MCP |
| [SQL Query Engine](docs/sql.md) | In-engine SQL fact tables and analytical TVFs (for developer tools and integrations) |

### Developer & Architecture Specifications
| Document | Description |
| --- | --- |
| [Architecture](docs/architecture.md) | Crate boundaries, dataflow diagram, asset/texture pipelines, and WASM bridge |
| [Invariants](docs/invariants.md) | Property-tested formal invariants (I1–I9) |
| [Libraries](docs/libraries.md) | Dependency selection bar and locked crate choices |
| [JSON Schema](docs/json-schema.md) | Shared result and option schema contracts |

---

## 🛠️ Development

### Prerequisites
* Rust stable (2021 edition)
* Node.js (v18+) for the web UI

### Building & Testing
```bash
# Set up pre-commit hooks (fmt, clippy, web tsc)
git config core.hooksPath .githooks

# Run workspace Rust tests & MCP smoke check
cargo test --workspace
./scripts/mcp-smoke.sh

# Run Web UI locally
cd web
npm install
npm run build:wasm
npm run build:defs
npm run dev
```

---

## 📄 License

This project is licensed under [AGPL-3.0](LICENSE).  
*Victoria 3 is a trademark of Paradox Interactive AB. This project is not affiliated with or endorsed by Paradox Interactive.*
