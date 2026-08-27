# Changelog

All notable changes to **Victoria 3 Analyzer** are documented in this file.
This project adheres to [Semantic Versioning](https://semver.org/).

---

## [Unreleased]

### 🌟 Added / Changed
<<<<<<< HEAD
- **Country and state naming:** Countries use `name` (tag) and `label` (display
  name). States keep `region_name` / `region_label` for shared geography and use
  `state_label` for the owned-slice display name. SQL `states` / `countries`
  columns match. **Breaking** for JSON, SQL, and web.
=======
- **Geo/country naming:** `StateInfo` uses `region_name` / `region_label` /
  `label`; `CountryInfo` uses `name` / `label` (int `id`
  kept). SQL `states` / `countries` columns match. **Breaking** for JSON, SQL,
  and web.
>>>>>>> bb60ab9 (refactor: use bare name/label on CountryInfo and StateInfo)

- **Pop / need / profession naming:** Rows that are only about one profession or
  need (`ProfessionCount`, `PopNeedBasket`, `StateNeed`, `StateQualification`,
  `ProfessionGap`) now use `name` (script key) and `label` (display text). Rows
  that mix several things keep prefixes (`StatePop.profession_*` /
  `culture_*`; SQL `building_staffing` keeps `profession_name` /
  `profession_label`). **Breaking** for JSON, SQL, and web.

- **Goods naming:** Goods rows (`GoodPrice` / `GoodFlow` / `StateGood`) use
  `name` (script key) / `label` (display text); SQL `goods` uses the same.
  Mixed IO lists still use `{good_name, good_label, qty}`. Alert and mitigation
  good script keys are `good_name`. **Breaking** for JSON, SQL, and web.

- **Dense identity types:** Renamed `GoodIdx` → `GoodId` and `NeedIdx` → `NeedId`
  across defs, prices, planning, and SQL. **Breaking** for Rust callers of those
  newtypes (transparent `u16`; postcard layout unchanged).

- **Planning `SimpleSubgoal`:** Renamed goal leaf type `Atom` → `SimpleSubgoal`,
  `Goal::Atom` → `Goal::Simple`, `atoms()` → `simple_subgoals()`, and related
  helpers (`AtomStatus` → `SimpleSubgoalStatus`, `has_*_atom` →
  `has_*_simple_subgoal`). Compare `GapDiff.atom` → `simple_subgoal` (JSON +
  TypeScript). Docs define **simple subgoal** as “simple in this compile,” not
  forever irreducible. **Breaking** for `vic3-planning` / compare JSON / web
  types.
- **Follow-on naming:** `successors_for_atoms` → `successors_for_simple_subgoals`,
  `expand_tech_gap_atoms` → `expand_tech_gap_simple_subgoals` (and private
  helpers). **Breaking** for those `vic3-planning` call sites.


- **Planning `ConstructionSlots`:** `max_parallel_construction_jobs` now returns
  `ConstructionSlots` (`NonZeroUsize` newtype, always ≥ 1) instead of `usize`.
  Call sites need `.get()` (or `Into<usize>`) for a raw count. **Breaking** for
  external `vic3-planning` consumers of that API.

---

## [0.7.0] - 2026-08-22

### 🌟 Added / Changed
- **UI Refresh:** Restored legacy light theme aesthetic to the Tauri desktop app, removing the injected dark SPA styles for improved visual consistency.
- **Documentation Integrity:** Updated all user-facing documentation screenshots to reflect the new UI aesthetic.
- **Testing Improvements:** Fixed the desktop UI screenshot capture pipeline (mock API logic and page wait selectors) to work properly with Tauri application architecture changes.

---

## [0.6.0] - 2026-08-22

### 🌟 Added
- **Tauri Dashboard Command Center**:
  - Replaced cluttered developer diagnostic status tiles with a friendly onboarding welcome and clear CTA when no save is loaded.
  - When a save is active, displays an interactive command center with active country metadata (`player_tag()`, `army_power()`), save details, and quick summary tables.
  - Added live Top Alerts and Top Shortages tables with interactive deep-linking (clicking a good in the shortage list immediately opens the **Prices** pane focused on that good).
- **Screenshot Harness & Documentation Drift Guard**:
  - Added automated Playwright-based screenshot verification for desktop UI goldens (`desktop-dashboard.png`, `desktop-alert-mitigations.png`).
  - Added explicit developer warnings in `compare.mjs` to remind contributors to review and update player documentation whenever UI layouts change.
- **Cross-Platform End-to-End Testing**:
  - Introduced WebdriverIO test suites covering Web and native Tauri desktop companion workflows.

### 🐛 Fixed
- Fixed save loading in the desktop companion where `refreshSaves()` was clearing the load status message.
- Fixed unhandled JS exception on startup caused by removed diagnostic DOM references.
- Fixed WebdriverIO v9 matcher compatibility and Vite base URL routing in E2E testing.

---

## [0.5.0] - 2026-08-22

### 🌟 Added
- **Automated Desktop Release Pipeline**:
  - Multi-platform packaging workflow in GitHub Actions building native desktop companion executables.
  - Universal macOS DMG (Apple Silicon & Intel), Windows x64 setup installer, and Linux binaries (`.AppImage`, `.deb`, `.rpm`).
- **Player Documentation Overhaul**:
  - Restructured player-facing vs developer documentation.
  - Polished copy across all analysis panes, guidance text, and installation walkthroughs.

---

## [0.4.0] - 2026-08-22

### 🌟 Added
- **AI Strategic Co-Pilot (MCP)**:
  - Model Context Protocol integration enabling LLMs (Claude Desktop, Cursor, Gemini, ChatGPT) to inspect campaign state, diagnose domestic shortages, and preview what-if adjustments.
- **Player Showcase & Visual Assets**:
  - Revamped README with feature walkthroughs, gameplay use cases, and locked high-resolution UI screenshots.
  - Playwright screenshot capture harness under `scripts/docs-screenshots` for reproducible docs assets.

---

## [0.3.0] - 2026-08-22

### 🌟 Added
- **Initial Developer Preview Release**:
  - Offline non-linear market price equilibrium solver accounting for pop wealth, substitution shares, and MAPI access.
  - What-If Economic Simulator for previewing building expansions and PM changes before committing in-game.
  - Strategic Goal Planning engine powered by A* search to plan expansion and research sequences.
  - DataFusion SQL query engine with custom table-valued functions (`alerts()`, `plan()`, `suggest_mitigations()`).
  - Browser-based WebAssembly client with 100% private client-side parsing and IndexedDB timeline branching.
  - Native Tauri desktop companion with local and Steam Cloud save auto-discovery.
