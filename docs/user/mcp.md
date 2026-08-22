# Ask your AI (MCP)

Talk to your local [Victoria 3](https://www.paradoxinteractive.com/games/victoria-3/about) autosave from Cursor, Claude Desktop, or any stdio MCP client ([wiki](https://vic3.paradoxwikis.com/)). Run `vic3-analyzer mcp`: the model binds a save, reads `campaign_brief`, then uses `query` / `preview_delta`.

Numbers in the [example trajectories](#example-trajectories) come from the in-repo **fixture campaign** (plaintext autosave + fixture defs).[^fixture] Your live Steam save will differ; the *shape* of the conversation is what to copy.

## Setup

Add an MCP server entry (Cursor: `~/.cursor/mcp.json`; Claude Desktop: its MCP servers settings) that runs the same binary as the [desktop companion](desktop.md), with args `["mcp"]`:

```json
{
  "mcpServers": {
    "vic3-analyzer": {
      "command": "/Applications/Vic3 Analyzer.app/Contents/MacOS/vic3-analyzer",
      "args": ["mcp"]
    }
  }
}
```

Use an **absolute** `command` path. On Linux/Windows, substitute whatever `which vic3-analyzer` (or your install location) returns. Config and save dirs are shared with the GUI.

If the first tool calls fail:

- Open the desktop app once (or set game/save paths in its Settings) so defs and the catalog exist.
- Call `refresh_catalog` if the save list is empty.
- Prefer plaintext saves when you can ([Save-game editing](https://vic3.paradoxwikis.com/Save-game_editing)).

Full client notes: [docs/mcp.md](../mcp.md).
## Flow

Always bind before fact tables or planning TVFs. Prefer this order:

1. **`use_save`** — stub name (`autosave`) or selector (`latest_autosave` / `latest` / `latest_named`). No filesystem paths.
2. **`campaign_brief`** — session meta, top domestic shortages, region×good hotspots, player-scoped alert kinds.
3. **`query`** / **`preview_delta`** — deeper SQL ([docs/sql.md](../sql.md)) or a building what-if (does not mutate the session).

```text
use_save: { "selector": "latest_autosave" }
campaign_brief: {}
query:  SELECT s.region_name, g.good, g.shortage, g.price
        FROM states s JOIN goods_by_state g USING (state_id)
        WHERE g.shortage > 0
        ORDER BY g.shortage DESC LIMIT 20
preview_delta: { "building": "building_rye_farm", "extra_levels": 1 }
```

Do **not** use `SELECT set_active_save(...)`. Short table names are already player-scoped; use `world_*` / `alerts('all')` only when you want the full save.

## Example questions

### Market triage

- What’s short in my market right now, and where are the hotspots?
- Why is coal expensive / alerting, and what mitigations does the model suggest?
- Which states are underemployed, and what qualifications are bottlenecks?
- Summarize player alerts by kind and severity.

### What-if

- If I add one level of rye farm, what happens to wood / grain prices and shortages?
- Preview expanding a trade center vs a local producer for this shortage.
- Warm-start a PM switch on building *N* and show only goods that moved.

### Plans & readiness

- Am I ready to declare war on Alsace? What gaps remain?
- Shortest model path to a higher GDP target.
- Research plan for nitroglycerin (and its prereqs).
- How do I raise army power projection (staffed barracks, not empty levels)?
- Will I default — what’s my credit headroom / solvency under the payday model?
- Qualifications SQL: where is farmer / engineer stock short of jobs?

Goal language and what A\* can close today: [docs/dsl.md](../dsl.md). Method limits: [prices](../prices.md), [planning](../planning.md), [_model-notes.md](_model-notes.md).

## Built-in prompts

The server ships reusable prompt templates — ask the client to use them, or paste their intent:

| Prompt | Starts with |
| --- | --- |
| `investigate_shortages` | `use_save` → `campaign_brief` → domestic shortage SQL |
| `compare_latest_autosave` | refresh catalog, bind latest autosave, summarize |
| `military_readiness` | military / formations queries when available |
| `what_is_loaded` | `vic3://session` + catalog counts |
| `plan_research` | `plan('research(tech=nitroglycerin)')` (swap the tech id) |

## Example trajectories

Format: User → Assistant → tool → result → Assistant.

### 1. Shortage triage

```text
User: What’s short in my market, and should I worry?

Assistant: Binding latest autosave, then campaign_brief and the top shortage row.

→ use_save { "selector": "latest_autosave" }
← { "name": "autosave", "kind": "autosave", "in_game_date": "1836.1.1",
    "country": "GER", "loaded": true }

→ campaign_brief {}
← {
    "player_tag": "GER",
    "top_goods": [{ "good": "coal", "shortage": 0.243 }],
    "hotspots": [{ "region_name": "Brandenburg", "good": "coal", "shortage": 0.243 }],
    "alert_kinds": {
      "goods_shortage": 1, "needs_unmet": 1,
      "underemployed": 1, "unfilled_education": 1
    }
  }

→ query {
    "sql": "SELECT s.region_name, g.good, g.shortage, g.price
            FROM states s JOIN goods_by_state g USING (state_id)
            WHERE g.shortage > 0
            ORDER BY g.shortage DESC LIMIT 20"
  }
← columns [region_name, good, shortage, price]
   rows [["Brandenburg", "coal", 0.243, 52.50]]

→ query {
    "sql": "SELECT evidence, mitigations FROM alerts()
            WHERE id = 'goods_shortage:coal'"
  }
← evidence: Buy 0.24 / Sell 0 / Price 52.50 vs base 30
   mitigations: build trade center; expand local coal producer
                (heuristic — not sized-to-fix)

Assistant: Only coal is short (~0.24 buy−sell in Brandenburg) at ~52.5 vs a 30 base.
  Severity-1 alerts agree. Alert mitigations rank trade center vs local coal;
  verify with preview_delta before you build. Fixture snapshot: coal only, not a cascade.
```

### 2. Building what-if (`preview_delta`)

```text
User: If I add one rye-farm level, does wood get cheaper?

Assistant: Binding, then preview_delta for +1 rye farm (read-only; session unchanged).

→ use_save { "name": "autosave" }
← loaded GER / 1836.1.1

→ preview_delta { "building": "building_rye_farm", "extra_levels": 1 }
← {
    "status": "converged",
    "residual": 9.6e-9,
    "applied": { "extra_levels": [
      { "building": "building_rye_farm", "extra_levels": 1 }
    ]},
    "goods": [{
      "id": "wood",
      "price_before": 15.40, "price_after": 5.60, "d_price": -9.80,
      "shortage_before": 0, "shortage_after": 0, "d_shortage": 0
    }],
    "limitations": [
      "Employment, wages, and trade volumes are frozen except explicit what-if deltas.",
      "…"
    ]
  }

Assistant: Model converged. Wood drops ~15.4 → ~5.6 (forestry PM on rye farm in
  this fixture). No shortage delta. Labor/trade frozen unless in the delta.[^model]
```

### 3. Prepare for war (`gaps` on `declare-war`)

```text
User: Can I declare war for Alsace yet?

Assistant: Checking gaps('declare-war(state=alsace)') — every conjunct must clear.

→ use_save { "selector": "latest_autosave" }
← loaded

→ query {
    "sql": "SELECT predicate, status, detail
            FROM gaps('declare-war(state=alsace)')"
  }
← rows:
   interest_in(state=alsace)           failing
   army_power_projection >= 100        unknown
   good_price(ammunition) <= 40        unknown
   solvent                             cleared

Assistant: Not ready on this snapshot. Interest in Alsace is failing (queue /
  wait interest). Army PP and ammo price are unknown in the save IR — unknown is
  not “zero army,” so don’t invent a muster. Solvency already clears. Next moves:
  declare interest, ensure barracks are staffed once PP is projected, and check
  munitions once that good exists in the solve. Full conjunct list: dsl.md.
```

### 4. GDP optimal path (`plan gdp`)

```text
User: Shortest path to gdp >= 1000 under the model?

Assistant: Gaps first, then plan() for an A* timeline.

→ use_save { "name": "autosave" }
← loaded

→ query {
    "sql": "SELECT predicate, status FROM gaps('gdp >= 1000')"
  }
← [["gdp >= 1000", "failing"]]

→ query {
    "sql": "SELECT step, day, action, detail
            FROM plan('gdp >= 1000') ORDER BY step"
  }
← (excerpt)
   0   0     QueueBuildingLevel   building_rye_farm
   1   200   WaitForEvent         BuildingCompleted rye_farm
   2   200   QueueBuildingLevel   building_rye_farm
   …   …     …                    more rye levels
   13  1200  SwitchPm             building_id=1 → pm_simple_farming
   14  1400  WaitForEvent         final BuildingCompleted

Assistant: On the fixture, the planner stacks rye-farm levels (~200 model days
  each) and finishes with a PM switch — about 1400 days to the target. Live
  campaigns usually pick higher-value industry; here rye is the top output lever
  the toy defs expose. GDP in plan() is modeled building output value, not the
  ledger ticker.[^model]
```

### 5. Research plan (nitroglycerin)

```text
User: Plan research to unlock nitroglycerin.

Assistant: Queueing plan('research(tech=nitroglycerin)').

→ use_save { "selector": "latest" }
← loaded

→ query {
    "sql": "SELECT step, day, action, detail
            FROM plan('research(tech=nitroglycerin)')
            ORDER BY step"
  }
←
   0    0    QueueTech      manufacturies
   1   50    WaitForEvent   TechCompleted manufacturies
   2   50    QueueTech      shaft_mining
   3  125    WaitForEvent   TechCompleted shaft_mining
   4  125    QueueTech      nitroglycerin
   5  225    WaitForEvent   TechCompleted nitroglycerin

Assistant: Unlocked on model day 225 via manufacturies → shaft_mining → nitroglycerin.
  For “already researched?” use gaps first; swap the tech id for other targets.
```

### 6. Credit / avoid default

```text
User: Am I about to default? What should I do?

Assistant: Checking credit_headroom and solvent.

→ use_save { "name": "autosave" }
← loaded

→ query {
    "sql": "SELECT predicate, status, detail
            FROM gaps('credit_headroom > 0')"
  }
← [["credit_headroom > 0", "cleared", …]]

→ query {
    "sql": "SELECT predicate, status FROM gaps('solvent')"
  }
← [["solvent", "cleared"]]

→ query {
    "sql": "SELECT step, day, action, detail
            FROM plan('credit_headroom > 0') ORDER BY step"
  }
← (empty plan — goal already true)

Assistant: On this fixture you’re already solvent with positive credit headroom,
  so plan() returns no steps. On a live save with failing headroom, expect
  payday / tax actions under the compact model — not a full investment-pool
  sim.[^model] Pair with weekly_balance >= … for the income side.
```

## Technical reference

| Doc | Role |
| --- | --- |
| [docs/mcp.md](../mcp.md) | Tools, resources, prompts, smoke client |
| [docs/sql.md](../sql.md) | Tables, UDFs, `plan` / `gaps`, read-only rules |
| [docs/dsl.md](../dsl.md) | Goal language and closability |

Related player surfaces: [desktop](desktop.md) · [web](web.md) · [overview](README.md).

## Model notes

Prices/plans are model-scoped; labor/trade stay frozen except explicit deltas. Details: [_model-notes.md](_model-notes.md).[^model][^pops]

---

[^model]: Method details: [prices](../prices.md), [planning](../planning.md), and [_model-notes.md](_model-notes.md). Frozen labor/trade in solves; MAPI/market access simplified; planner coverage uneven (research/GDP/army PP/solvency close more often than SoL).
[^pops]: Not a closed-loop pop or migration planner. Qualifications SQL (`state_qualifications`, `building_staffing`, education alerts) is save-backed triage — A\* does not accrue monthly qualifications as its engine.
[^fixture]: Trajectory numbers from `crates/vic3-load/tests/fixtures/plaintext.txt` + fixture defs blob via the [MCP smoke](../mcp.md#smoke-client) setup (`use_save` name `autosave`, GER 1836.1.1). Labeled **fixture campaign** throughout.
