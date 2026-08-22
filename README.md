# vic3-analyzer

Local advisor for [Victoria 3](https://www.paradoxinteractive.com/games/victoria-3/about) ([wiki](https://vic3.paradoxwikis.com/)): triage market shortages, run building what-ifs, search **goal plans** (war readiness, GDP paths, research), and drive the same engine from SQL or an LLM over MCP. Your saves and game definitions **never leave your machine**.

![Prices workspace — market overview with goods icons](docs/assets/web-prices.png)

![Advanced Query — shortage SQL for player-owned states](docs/assets/desktop-query-shortages.png)

## Why it’s hard

Vic3 decisions look like spreadsheets (“build more steel”) and are not.

Construction sectors eat iron, wood, and tools — a feedback loop, not a static cost line. [Shortages](https://vic3.paradoxwikis.com/Market#Shortage) cascade into throughput penalties. Buildings and pops pay **local** prices (a blend of the national [market](https://vic3.paradoxwikis.com/Market) price and the state’s isolated price — [Market Access Price Impact (MAPI)](https://vic3.paradoxwikis.com/Market#Local_prices), reduced further by low [market access](https://vic3.paradoxwikis.com/Infrastructure#Market_access)). Advanced industry needs [qualifications](https://vic3.paradoxwikis.com/Profession#Qualifications); peasants do not become engineers overnight. War readiness is a checklist (declared interest, staffed army power, munitions, solvency), not a single button. Order matters across months of construction, hiring, and research.

A save viewer shows the ledger. This stack re-prices *your* market after a change and searches build/research sequences under staffing and fiscal constraints — under our model. Fuller pitch: [docs/user/README.md](docs/user/README.md).

## Who it’s for

Mid/late-campaign players who already feel construction-price opacity, empty factories, and “what should I queue next?” paralysis. Not a replacement for learning the game — start from the [Victoria 3 Wiki](https://vic3.paradoxwikis.com/) for the rules.

## Three ways to use it

### Ask your AI (MCP)

Point Cursor or Claude Desktop at `vic3-analyzer mcp`. The model binds your autosave, reads `campaign_brief`, then runs `query` / `preview_delta` — same dialect as Advanced Query.

```text
You: What’s short in my market, and where?

→ use_save { "selector": "latest_autosave" }
→ campaign_brief {}
→ query  SELECT s.region_name, g.good, g.shortage, g.price
         FROM states s JOIN goods_by_state g USING (state_id)
         WHERE g.shortage > 0
         ORDER BY g.shortage DESC LIMIT 20
```

Full trajectories (shortage triage, what-if, war gaps, GDP path, research, credit): [docs/user/mcp.md](docs/user/mcp.md).

### Desktop companion

Native shell that finds Steam/Paradox paths, binds stubs by name, runs Advanced Query, drills into States/Prices, and opens Timeline for a sequenced plan. Building-level what-if lives on the web and via MCP `preview_delta` today.

![Dashboard — save catalog and defs status](docs/assets/desktop-dashboard.png)

![Timeline — time-ordered GDP / research plan steps](docs/assets/desktop-timeline-gdp.png)

Guide: [docs/user/desktop.md](docs/user/desktop.md).

### Browser workbench

Zero-install [live demo](https://jmalicki.github.io/vic3-analyzer/). Drop a save (nothing uploads), explore prices, stress-test a build with What-if, then Goal gaps / Timeline for war and GDP paths.

Paradox game data is **copyrighted** — the site ships no defs. Drag your local `Victoria 3/game` folder once; files stay in the browser (IndexedDB).

![What-if — re-solve prices after a building delta](docs/assets/web-what-if.png)

![Goal gaps — prepare-for-war checklist under the model](docs/assets/web-gaps-war.png)

Guide: [docs/user/web.md](docs/user/web.md).

## Try it in 5 minutes

1. Prefer plaintext saves: `"save_file_format": "zip_text_all"` in `pdx_settings.json`, then re-save ([Save-game editing](https://vic3.paradoxwikis.com/Save-game_editing)).
2. **Browser:** open the [live demo](https://jmalicki.github.io/vic3-analyzer/), drag your `Victoria 3/game` folder, drop a `.v3`.
3. **Desktop / MCP:** build and run the companion; MCP uses the same binary with args `["mcp"]`:

```json
{
  "mcpServers": {
    "vic3-analyzer": {
      "command": "/absolute/path/to/vic3-analyzer",
      "args": ["mcp"]
    }
  }
}
```

## What you can answer

- What’s short in my market, and where?
- If I add five steel mills / rye farms, what moves?
- Prepare for war over this state — what’s missing?
- Optimal path toward a GDP target / research plan under the model?
- Am I about to default — credit headroom?
- Where are qualifications bottlenecks before I spam factories?

## Docs

### Play

| Doc | Contents |
| --- | --- |
| [Play guides](docs/user/README.md) | Overview, why it’s hard, choose a surface |
| [MCP](docs/user/mcp.md) | LLM trajectories and tool flow |
| [Desktop](docs/user/desktop.md) | Companion UI and SQL recipes |
| [Web](docs/user/web.md) | Browser workbench and defs setup |

### Reference

| Doc | Contents |
| --- | --- |
| [sql](docs/sql.md) | DataFusion tables / UDFs / `plan()` |
| [dsl](docs/dsl.md) | Goal language |
| [prices](docs/prices.md) | Equilibrium method and limitations |
| [json-schema](docs/json-schema.md) | Result/option contract |
| [archive](docs/archive.md) | Past saves and alternative plans |

### Develop

| Doc | Contents |
| --- | --- |
| [architecture](docs/architecture.md) | Crates, data flow |
| [usage](docs/usage.md) | CLI / shared option structs |
| [planning](docs/planning.md) | `PlanningState`, A* |
| [mcp](docs/mcp.md) | MCP protocol reference |
| [desktop](docs/desktop.md) | Tauri config / auto-detect |
| [libraries](docs/libraries.md) | Locked deps |
| [invariants](docs/invariants.md) | Property tests |

```text
git config core.hooksPath .githooks
cargo test --workspace
cargo run -p vic3-analyzer            # companion UI
cargo run -p vic3-analyzer -- mcp     # stdio MCP
cd web && npm install && npm run build:wasm && npm run build:defs && npm run dev
```

Also: CLI via `vic3-cli` ([usage](docs/usage.md)). Screenshot regen: `scripts/docs-screenshots`.

## License

[AGPL-3.0](LICENSE) covers **our** code. Game definitions remain Paradox’s / yours — we never redistribute them.

## Model notes

See [docs/user/_model-notes.md](docs/user/_model-notes.md): frozen labor/trade in solves, not a full pop/migration planner, simplified MAPI/access, uneven planner coverage.
