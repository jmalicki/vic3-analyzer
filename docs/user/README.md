# Play guides

**vic3-analyzer** is a local Victoria 3 advisor: market triage, building what-ifs, and goal plans for war readiness, GDP paths, and research — under our model, not Paradox’s binary. Your saves and game definitions stay on your machine.

These guides are for mid/late-campaign players who already feel construction-price opacity, empty factories, and “what should I queue next?” paralysis. They are not a replacement for learning the game.

## Choose your surface

| Surface | Guide | When to use it |
| --- | --- | --- |
| **Ask your AI (MCP)** | [mcp.md](mcp.md) | Chat-driven investigation of your autosave in Cursor or Claude |
| **Desktop companion** | [desktop.md](desktop.md) | Local catalog, Advanced Query, States/Prices drill-down, Timeline |
| **Browser workbench** | [web.md](web.md) | Zero-install demo; What-if + Timeline / Goal gaps |

Same analysis stack underneath — pick the shell that matches how you play.

## Game data you provide

Analysis needs definitions from **your** Victoria 3 install (goods, buildings, production methods, localization, icons). Paradox game data is **copyrighted** — we never redistribute `common/`, English loc, DDS icons, or a derived defs blob.

- **Desktop / MCP** usually auto-detect Steam and Paradox save paths.
- **Web** cannot ship or fetch the install for you; you point the browser at your local `game` folder once (nothing uploads).

Prefer plaintext saves when you can: set `"save_file_format": "zip_text_all"` in `pdx_settings.json` and re-save. See [Save-game editing](https://vic3.paradoxwikis.com/Save-game_editing).

## Why it’s hard

Vic3 decisions look like spreadsheets (“build more steel”) and are not.

- **[Professions](https://vic3.paradoxwikis.com/Profession) / qualifications** — Peasants do not teleport into advanced industry. Pops need [qualifications](https://vic3.paradoxwikis.com/Profession#Qualifications) (wealth, literacy, current job, laws) before they can staff engineer and machinist jobs. Rural employment and [standard of living](https://vic3.paradoxwikis.com/Standard_of_living) feed that pipeline; universities and education access accelerate it. Spamming factories into an unqualified state just creates empty buildings and [employment](https://vic3.paradoxwikis.com/Building) shortages.
- **Construction goods feedback** — Expanding [construction sectors](https://vic3.paradoxwikis.com/Building#Construction_sector) raises demand for wood, iron, tools, and friends. That moves prices, which changes whether the next build is affordable — a loop, not a static cost line on a ledger.
- **[Shortages](https://vic3.paradoxwikis.com/Market#Shortage) cascade** — When buy orders roughly double sell orders, throughput penalties escalate. One short input wrecks the chain for weeks.
- **Local prices lie to your eyes** — Buildings and pops pay **local** prices, not the national ticker. Local prices blend the national [market](https://vic3.paradoxwikis.com/Market) price with what the state would pay in isolation; that blend weight is [Market Access Price Impact (MAPI)](https://vic3.paradoxwikis.com/Market#Local_prices), reduced further by low [market access](https://vic3.paradoxwikis.com/Infrastructure#Market_access).
- **War readiness is a conjunction** — A [diplomatic play](https://vic3.paradoxwikis.com/Diplomatic_play) needs declared interest, staffed army power (not empty barracks levels), munitions prices under control, and solvency. Missing any one blocks the play.
- **Order matters** — Construction queues, hiring, research, and interest compete over months and years. Gut feel cannot re-solve a multi-good market after a hypothetical delta or search a combinatorial build/research graph while holding staffing and fiscal constraints in your head.

That is why a save viewer and a static calculator fall short — and why this project pairs a market solver, a goal planner (`gaps` / `plan` / Timeline), and optional LLM or SQL workbenches.

## Guides

| Guide | What you get |
| --- | --- |
| [Ask your AI (MCP)](mcp.md) | Example questions, tool flow, polished chat trajectories |
| [Desktop companion](desktop.md) | Screenshots, Advanced Query recipes, Timeline |
| [Browser workbench](web.md) | Defs setup, feature walkthrough, What-if vs Plans & gaps |

## Game terms

Quick wiki links used across these guides:

| Concept | Wiki |
| --- | --- |
| Market / local prices / MAPI / shortages | [Market](https://vic3.paradoxwikis.com/Market) |
| Market access / infrastructure | [Infrastructure](https://vic3.paradoxwikis.com/Infrastructure) |
| Buildings / construction / throughput | [Building](https://vic3.paradoxwikis.com/Building) |
| Budget / investment | [Budget](https://vic3.paradoxwikis.com/Budget) |
| Standard of living | [Standard of living](https://vic3.paradoxwikis.com/Standard_of_living) |
| Professions / qualifications | [Profession](https://vic3.paradoxwikis.com/Profession) |
| Technology / research | [Technology](https://vic3.paradoxwikis.com/Technology) |
| Diplomatic plays / interest | [Diplomatic play](https://vic3.paradoxwikis.com/Diplomatic_play) |
| Save format | [Save-game editing](https://vic3.paradoxwikis.com/Save-game_editing) |

## Model notes

See [_model-notes.md](_model-notes.md) for how our solver and planner differ from Paradox’s binary (frozen labor/trade in solves, simplified MAPI, uneven planner coverage, and more). Technical depth: [prices](../prices.md), [planning](../planning.md).
