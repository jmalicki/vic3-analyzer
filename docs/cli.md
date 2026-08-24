# CLI Reference & Batch Debugging

The command-line interface (`vic3-cli`) is a developer and power-user tool for headless batch price solves, qualification triage, what-if simulations, timeline planning, and plaintext save patching.

It operates directly over the core analysis engine ([`vic3-api`](../crates/vic3-api)) and emits structured JSON matching [`json-schema.md`](json-schema.md) when invoked with `--json`.

---

## Environment Variables

| Variable | Description | Notes |
| --- | --- | --- |
| `VIC3_SAVE` | Default path to a `.v3` save file | Used when `--save` is omitted |
| `VIC3_GAME` | Path to installed `Victoria 3/game` folder | Used to read raw game definitions |
| `VIC3_DEFS` | Path to a precompiled `defs.postcard` blob | Faster alternative to reading the live game directory |
| `VIC3_TOKENS` | Path to token mapping file | Required only for binary Ironman saves |

---

## Command Reference

### 1. Market Equilibrium: `prices`
Calculates national market prices and state-level blended [MAPI](https://vic3.paradoxwikis.com/Market#Market_access_price_impact) prices for all active goods.

```bash
cargo run -p vic3-cli -- prices \
  --save campaign.v3 \
  --game "/path/to/Victoria 3/game"
```

### 2. What-If Simulator: `what-if`
Simulates adding building levels or changing production methods and re-equilibrates prices.

```bash
# Add 5 levels of arms industries
cargo run -p vic3-cli -- what-if \
  --save campaign.v3 \
  --building arms_industry \
  --extra-levels 5

# Complex multi-building delta via JSON
cargo run -p vic3-cli -- what-if \
  --save campaign.v3 \
  --delta-json '{"production_methods": [{"building": "building_steel_mills", "method": "pm_bessemer_process"}]}'
```

### 3. Bottleneck Diagnostics: `alerts`
Audits pop employment shortages and outputs qualification promotion feeder recommendations.

```bash
cargo run -p vic3-cli -- alerts \
  --save campaign.v3 \
  --game "/path/to/Victoria 3/game"
```

### 4. Readiness Gaps: `gaps`
Evaluates boolean readiness conditions for a target goal defined in the [Goal DSL](dsl.md).

```bash
cargo run -p vic3-cli -- gaps \
  --save campaign.v3 \
  --goal "declare-war(state=alsace)"
```

### 5. Timeline Planning: `plan`
Finds an optimal action sequence (tech, laws, constructions) to achieve a goal.

```bash
cargo run -p vic3-cli -- plan \
  --save campaign.v3 \
  --goal "research(tech=nitroglycerin)" \
  --label "rush explosives"
```

### 6. PM Optimization: `optimize-pms`
Evaluates production method configurations to optimize along a target axis (`income`, `productivity`, or `sol`).

```bash
cargo run -p vic3-cli -- optimize-pms \
  --save campaign.v3 \
  --axis income
```

### 7. Plaintext Save Patching: `export-save`
Applies simulated what-if modifications surgically to a plaintext `.v3` save file.

```bash
cargo run -p vic3-cli -- export-save \
  --save campaign.v3 \
  --delta-json '{"production_methods": [...]}' \
  --out modified.v3
```

### 8. Definitions Blob Compilation: `defs export`
Precompiles allowlisted game definitions and DDS top-mip icons into a portable `defs.postcard` blob for testing or fast CI execution.

```bash
cargo run -p vic3-cli -- defs export \
  --game "/path/to/Victoria 3/game" \
  --out defs.postcard
```

### 9. Local Archive Management: `archive`
Inspects, lists, and diffs historical plan records stored in `$XDG_DATA_HOME/vic3-analyzer/`.

```bash
# List all archived records
cargo run -p vic3-cli -- archive list

# Show detailed record JSON
cargo run -p vic3-cli -- archive show <record-id>

# Diff two plan or price solves
cargo run -p vic3-cli -- archive diff <record-id-1> <record-id-2>
```

### 10. AI Companion & MCP Integration: `mcp`
Inspects, installs, configures, or serves the Model Context Protocol (MCP) server for desktop AI assistants (Claude Desktop, LM Studio, Windows Copilot, Codex, Cursor, Claude Code).

```bash
# Interactive installation (presents checkboxes for detected apps in your terminal):
cargo run -p vic3-cli -- mcp install

# Automated non-interactive installation to all detected apps:
cargo run -p vic3-cli -- mcp install -y

# Configure a specific application:
cargo run -p vic3-cli -- mcp install --client claude-desktop
cargo run -p vic3-cli -- mcp install --client lm-studio

# Preview changes without modifying disk:
cargo run -p vic3-cli -- mcp install --dry-run

# Inspect integration status across all supported desktop AI applications:
cargo run -p vic3-cli -- mcp status

# Remove vic3-analyzer from client configs (leaves other tools untouched):
cargo run -p vic3-cli -- mcp uninstall --client lm-studio
cargo run -p vic3-cli -- mcp uninstall --all

# Print configuration snippet for manual pasting:
cargo run -p vic3-cli -- mcp show-config --client claude-desktop
cargo run -p vic3-cli -- mcp show-config --client codex

# Run stdio MCP server directly from the CLI:
cargo run -p vic3-cli -- mcp serve
```

---

## CPU Profiling

For release-speed binaries with debug info (symbols + frame pointers) suitable for sampling profilers such as [samply](https://github.com/mstange/samply):

```bash
RUSTFLAGS='-C force-frame-pointers=yes' cargo build --profile profiling -p vic3-cli
```

---

## Machine-Readable Output (`--json`)

Passing `--json` to any command (e.g. `prices`, `plan`, `mcp status`) outputs clean, unformatted NDJSON/JSON stdout suitable for piping to `jq` or integrating into custom scripts. Errors and diagnostic progress are emitted to `stderr`.
