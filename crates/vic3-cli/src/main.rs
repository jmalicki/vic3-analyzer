//! Victoria 3 analyzer CLI: clap facade over the shared analysis stack.
//!
//! # Role
//!
//! **clap lives only in this crate.** Inner option structs
//! ([`vic3_prices::SolveOpts`], [`vic3_prices::WhatIfOpts`], [`vic3_planning::PlanOpts`], …)
//! have no `PathBuf`; filesystem fields sit on clap wrappers (`IoArgs`, `*Cli`).
//! wasm never links clap.
//!
//! JSON shapes match `vic3-api` so CLI `--json`, wasm, Tauri, and MCP stay aligned.
//! This binary currently loads a [`vic3_load::WorldSave`] and calls
//! `vic3-prices` / `vic3-planning` directly for speed; path helpers and session APIs
//! in `vic3-api` are what Tauri / MCP / wasm use for the same results.
//!
//! # Command → analysis mapping
//!
//! | Subcommand | `vic3-api` counterpart | Notes |
//! | --- | --- | --- |
//! | `prices` | `prices_json` / `prices_from_paths` | table or `--json` [`PricesResult`] |
//! | `what-if` | `what_if_json` | building + `extra_levels` |
//! | `alerts` | `alerts_json` | after baseline solve |
//! | `mutate` | `loaded_apply_delta_json` (preview) | `--delta-json` [`WorldDelta`]; no file write |
//! | `optimize-pms` | `loaded_optimize_pms_json` | `--axis` income / productivity / sol |
//! | `export-save` | `export_save_bytes` | writes `--out` only; never overwrites `--save` |
//! | `gaps` | `gaps_json` | `{ satisfied, gaps, limitations }` |
//! | `plan` | `plan_json` | also archives under XDG |
//! | `defs export` | `defs_blob_from_game` | local postcard; do not publish |
//! | `archive …` | `vic3-planning` records | list / show / diff / import / export |
//!
//! Shared IO: `--save` / `VIC3_SAVE`, optional `--tokens` / `VIC3_TOKENS`,
//! `--game` / `VIC3_GAME` or `--defs` / `VIC3_DEFS`.
//!
//! `prices` / `what-if` / `alerts` / `mutate` / `optimize-pms` load `WorldSave`
//! (not the full file-shaped `Save`) and still `solve` a complete `PricesResult`.
//! The default table only prints goods; compact pop rows exist for the webapp.
//!
//! See `docs/cli.md`.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use sha2::{Digest, Sha256};
use vic3_defs::GameDefs;
use vic3_load::{empty_tokens, export_save, load_path_world, load_tokens_path, SavePatch};
use vic3_planning::PlanningState;
use vic3_planning::SimpleSubgoal;
use vic3_planning::{compare, AnalysisRecord, EconomyContext, PlanOpts, PlanResult};
use vic3_prices::{
    alerts, optimize_pms, preview, solve, what_if, AlertsResult, OptimizeAxis as PriceOptimizeAxis,
    OptimizeResult, PricesResult, SolveOpts, WhatIfOpts, World, WorldDelta,
};
use vic3save::PdsDate;

mod mcp_cli;

use mcp_cli::{run_mcp, McpCli};

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Prices(cmd) => {
            let (world, defs) = load_world(&cmd.io)?;
            emit(&solve(&world, &defs, cmd.solve.into()), cmd.json)
        }
        Commands::WhatIf(cmd) => {
            let (world, defs) = load_world(&cmd.io)?;
            emit(
                &what_if(
                    &world,
                    &defs,
                    &cmd.what_if.resolve(&defs)?,
                    cmd.solve.into(),
                ),
                cmd.json,
            )
        }
        Commands::Alerts(cmd) => run_alerts(cmd),
        Commands::Mutate(cmd) => run_mutate(cmd),
        Commands::OptimizePms(cmd) => run_optimize_pms(cmd),
        Commands::ExportSave(cmd) => run_export_save(cmd),
        Commands::Gaps(cmd) => run_gaps(cmd),
        Commands::Plan(cmd) => run_plan(cmd),
        Commands::Defs(cmd) => run_defs(cmd),
        Commands::Archive(cmd) => run_archive(cmd),
        Commands::Mcp(cmd) => run_mcp(cmd),
    }
}

/// Victoria 3 analyzer CLI.
#[derive(Debug, Parser)]
#[command(name = "vic3-cli", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Solve market prices (pop consumption in the loop; buildings frozen).
    Prices(PricesCli),
    /// Apply extra building levels and re-solve. Employment stays frozen.
    WhatIf(WhatIfCli),
    /// Diagnose shortages from a solved market (`AlertsResult`).
    Alerts(AlertsCli),
    /// Preview a [`WorldDelta`] and re-solve. Does not write a save.
    Mutate(MutateCli),
    /// Search production methods along income, productivity, or SoL.
    OptimizePms(OptimizePmsCli),
    /// Patch a plaintext `.v3` into a new file. Never overwrites `--save`.
    ExportSave(ExportSaveCli),
    /// Evaluate a goal and list its currently unsatisfied simple subgoals.
    Gaps(GapsCli),
    /// Find and archive a shortest goal-relevant action sequence.
    Plan(PlanCli),
    /// Build definition blobs for the browser UI.
    Defs(DefsCli),
    /// Browse local analysis records.
    Archive(ArchiveCli),
    /// Configure and manage the Model Context Protocol (MCP) server for AI assistants.
    Mcp(McpCli),
}

#[derive(Debug, Args)]
struct DefsCli {
    #[command(subcommand)]
    command: DefsCommand,
}

#[derive(Debug, Subcommand)]
enum DefsCommand {
    /// Encode a game install's definitions as a postcard blob for the web UI.
    ///
    /// The blob contains Paradox game data, so keep it local: do not commit it
    /// or publish it with the site.
    Export {
        /// Victoria 3 install, or a fixture tree with `common/` at the root.
        #[arg(long, env = "VIC3_GAME")]
        game: PathBuf,
        /// Destination file, conventionally `defs.postcard`.
        #[arg(long, short)]
        out: PathBuf,
    },
}

/// Filesystem inputs. Inner option structs (`SolveOpts`, `WhatIfOpts`) have no `PathBuf`.
#[derive(Debug, Args)]
struct IoArgs {
    /// Path to a `.v3` save (plaintext or binary).
    #[arg(long, env = "VIC3_SAVE")]
    save: PathBuf,
    /// Paradox token map. Required for binary (ironman) saves; omit for plaintext.
    #[arg(long, env = "VIC3_TOKENS")]
    tokens: Option<PathBuf>,
    /// Victoria 3 install, or a fixture tree with `common/` at the root.
    /// Required unless `--defs` is set.
    #[arg(long, env = "VIC3_GAME")]
    game: Option<PathBuf>,
    /// Postcard blob from `vic3-cli defs export`. Skips CoA compositing.
    #[arg(long, env = "VIC3_DEFS")]
    defs: Option<PathBuf>,
}

/// Flatten of [`SolveOpts`] — clap wrapper so wasm never links clap.
#[derive(Debug, Clone, Args)]
struct SolveArgs {
    /// Residual threshold for converged status (I5).
    #[arg(long, default_value_t = SolveOpts::default().residual_eps)]
    residual_eps: f64,
    /// Combined successive-substitution + Basin iteration cap.
    #[arg(long, default_value_t = SolveOpts::default().max_iters, value_parser = clap::value_parser!(u32).range(1..))]
    max_iters: u32,
}

impl From<SolveArgs> for SolveOpts {
    fn from(args: SolveArgs) -> Self {
        Self {
            residual_eps: args.residual_eps,
            max_iters: args.max_iters,
            warm_rel: None,
        }
    }
}

/// Flatten of [`WhatIfOpts`].
#[derive(Debug, Clone, Args)]
struct WhatIfArgs {
    /// Building type script id to bump (resolved to a dense index via defs).
    #[arg(long)]
    building: String,
    /// Non-negative extra levels added to matching buildings.
    #[arg(long)]
    extra_levels: u32,
}

impl WhatIfArgs {
    fn resolve(self, defs: &vic3_defs::GameDefs) -> Result<WhatIfOpts> {
        let building_type_id = defs
            .resolve_building_type_index(&self.building)
            .ok_or_else(|| anyhow::anyhow!("unknown building type `{}`", self.building))?;
        Ok(WhatIfOpts {
            building_type_id,
            extra_levels: self.extra_levels,
        })
    }
}

#[derive(Debug, Args)]
struct PricesCli {
    #[command(flatten)]
    io: IoArgs,
    /// Print [`PricesResult`] JSON (includes `limitations` and `residual`).
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    solve: SolveArgs,
}

#[derive(Debug, Args)]
struct WhatIfCli {
    #[command(flatten)]
    io: IoArgs,
    #[command(flatten)]
    what_if: WhatIfArgs,
    /// Print [`PricesResult`] JSON (includes `limitations` and `residual`).
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    solve: SolveArgs,
}

#[derive(Debug, Args)]
struct AlertsCli {
    #[command(flatten)]
    io: IoArgs,
    /// Print [`AlertsResult`] JSON.
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    solve: SolveArgs,
}

#[derive(Debug, Args)]
struct MutateCli {
    #[command(flatten)]
    io: IoArgs,
    /// JSON [`WorldDelta`] (extra levels, then production methods).
    #[arg(long)]
    delta_json: String,
    /// Print [`PricesResult`] JSON (includes `limitations` and `residual`).
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    solve: SolveArgs,
}

/// Objective for [`Commands::OptimizePms`]. Inner type has no `PathBuf`.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum OptimizeAxis {
    Income,
    Productivity,
    Sol,
}

#[derive(Debug, Args)]
struct OptimizePmsCli {
    #[command(flatten)]
    io: IoArgs,
    /// Rank candidate production methods by this axis.
    #[arg(long, value_enum)]
    axis: OptimizeAxis,
    /// Print [`OptimizeResult`] JSON (grouped PM changes plus a compact score delta).
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    solve: SolveArgs,
}

#[derive(Debug, Args)]
struct ExportSaveCli {
    /// Path to a `.v3` save (plaintext). Never overwritten.
    #[arg(long, env = "VIC3_SAVE")]
    save: PathBuf,
    /// JSON [`SavePatch`] (production methods and extra levels).
    #[arg(long)]
    delta_json: String,
    /// Destination `.v3`. Must be a different path from `--save`.
    #[arg(long)]
    out: PathBuf,
}

#[derive(Debug, Args)]
struct GapsCli {
    #[command(flatten)]
    io: IoArgs,
    /// Goal DSL expression to evaluate.
    #[arg(long)]
    goal: String,
    /// Print gap JSON (includes price-solve `limitations`).
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    solve: SolveArgs,
}

#[derive(Debug, Args)]
struct PlanCli {
    #[command(flatten)]
    io: IoArgs,
    /// Goal DSL expression to achieve.
    #[arg(long)]
    goal: String,
    /// Optional name for this alternative plan.
    #[arg(long)]
    label: Option<String>,
    /// Reject plans longer than this many days.
    #[arg(long, default_value_t = 3650)]
    max_days: u32,
    /// Allow zero-day production-method SwitchPm edges (off by default; no UI).
    #[arg(long, default_value_t = false)]
    allow_pm_changes: bool,
    /// Print PlanResult JSON.
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    solve: SolveArgs,
}

#[derive(Debug, Args)]
struct ArchiveCli {
    #[command(subcommand)]
    command: ArchiveCommand,
}

#[derive(Debug, Subcommand)]
enum ArchiveCommand {
    /// List archived analyses, newest first.
    List,
    /// Print one AnalysisRecord as JSON.
    Show { id: String },
    /// Compare two stored results without re-running analysis.
    Diff { left: String, right: String },
    /// Write one AnalysisRecord JSON file.
    Export { id: String, path: PathBuf },
    /// Add an AnalysisRecord JSON file to the local archive.
    Import { path: PathBuf },
}

#[derive(Debug, Serialize)]
struct GapsResult {
    satisfied: bool,
    gaps: Vec<SimpleSubgoal>,
    limitations: Vec<String>,
}

fn load_defs(io: &IoArgs) -> Result<GameDefs> {
    if let Some(path) = &io.defs {
        let bytes =
            fs::read(path).with_context(|| format!("reading defs blob {}", path.display()))?;
        return vic3_defs::decode_blob(&bytes)
            .with_context(|| format!("decoding defs blob {}", path.display()));
    }
    let game = io.game.as_ref().context("provide --game or --defs")?;
    vic3_defs::load_from_path(game).with_context(|| format!("loading defs from {}", game.display()))
}

fn load_world(io: &IoArgs) -> Result<(World, GameDefs)> {
    let defs = load_defs(io)?;
    // WorldSave skips markets / trade routes / construction queues. Unknown
    // keys in a one-member `gamestate` zip still inflate; this only avoids IR.
    let save = if let Some(tokens) = &io.tokens {
        let tokens = load_tokens_path(tokens)
            .with_context(|| format!("loading tokens from {}", tokens.display()))?;
        load_path_world(&io.save, tokens)
    } else {
        load_path_world(&io.save, empty_tokens())
    }
    .with_context(|| format!("loading save from {}", io.save.display()))?;
    let world = World::from_save(&save, &defs);
    drop(save);
    Ok((world, defs))
}

fn run_defs(cmd: DefsCli) -> Result<()> {
    let DefsCommand::Export { game, out } = cmd.command;
    let defs = vic3_defs::load_from_path(&game)
        .with_context(|| format!("loading defs from {}", game.display()))?;
    let blob = vic3_defs::encode_blob(&defs).context("encoding defs blob")?;
    if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(&out, &blob).with_context(|| format!("writing {}", out.display()))?;
    writeln!(
        io::stderr(),
        "wrote {} goods, {} bytes to {}",
        defs.goods.len(),
        blob.len(),
        out.display()
    )?;
    Ok(())
}

fn run_alerts(cmd: AlertsCli) -> Result<()> {
    let (world, defs) = load_world(&cmd.io)?;
    let prices = solve(&world, &defs, cmd.solve.into());
    emit_alerts(&alerts(&world, &defs, &prices), cmd.json)
}

fn run_mutate(cmd: MutateCli) -> Result<()> {
    let (world, defs) = load_world(&cmd.io)?;
    let delta: WorldDelta =
        serde_json::from_str(&cmd.delta_json).context("parsing --delta-json as WorldDelta")?;
    let mut opts: SolveOpts = cmd.solve.into();
    let baseline = solve(&world, &defs, opts.clone());
    if !baseline.relative.is_empty() {
        opts.warm_rel = Some(baseline.relative);
    }
    emit(&preview(&world, &defs, &delta, opts), cmd.json)
}

fn run_optimize_pms(cmd: OptimizePmsCli) -> Result<()> {
    let (world, defs) = load_world(&cmd.io)?;
    let opts: SolveOpts = cmd.solve.into();
    let baseline = solve(&world, &defs, opts.clone());
    let axis = match cmd.axis {
        OptimizeAxis::Income => PriceOptimizeAxis::Income,
        OptimizeAxis::Productivity => PriceOptimizeAxis::Productivity,
        OptimizeAxis::Sol => PriceOptimizeAxis::Sol,
    };
    emit_optimize(
        &optimize_pms(&world, &defs, &baseline, opts, axis),
        cmd.json,
    )
}

fn run_export_save(cmd: ExportSaveCli) -> Result<()> {
    anyhow::ensure!(
        !same_path(&cmd.save, &cmd.out),
        "--out must be a new path; refusing to overwrite --save"
    );
    let original =
        fs::read(&cmd.save).with_context(|| format!("reading save {}", cmd.save.display()))?;
    let patch: SavePatch =
        serde_json::from_str(&cmd.delta_json).context("parsing --delta-json as SavePatch")?;
    let patched = export_save(&original, &patch)?;
    if let Some(parent) = cmd
        .out
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(&cmd.out, &patched).with_context(|| format!("writing {}", cmd.out.display()))?;
    writeln!(
        io::stderr(),
        "wrote {} bytes to {}",
        patched.len(),
        cmd.out.display()
    )?;
    Ok(())
}

fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn run_gaps(cmd: GapsCli) -> Result<()> {
    let (world, defs) = load_world(&cmd.io)?;
    let prices = solve(&world, &defs, cmd.solve.into());
    let country_tag = world
        .player_country_tag()
        .context("save has no playable country")?;
    let state = PlanningState::from_world_with_prices(&world, country_tag, &prices, &defs)?;
    let goal = vic3_planning::parse(&cmd.goal)?;
    let mut limitations = prices.limitations;
    if goal.has_army_simple_subgoal() {
        state.push_army_power_limitation(&mut limitations);
    }
    let result = GapsResult {
        satisfied: vic3_planning::evaluate(&goal, &state),
        gaps: vic3_planning::gaps_with_defs(&goal, &state, &defs),
        limitations,
    };
    emit_gaps(&result, cmd.json)
}

fn run_plan(cmd: PlanCli) -> Result<()> {
    let save_bytes = fs::read(&cmd.io.save)
        .with_context(|| format!("reading save {}", cmd.io.save.display()))?;
    let (world, defs) = load_world(&cmd.io)?;
    let solve_opts: SolveOpts = cmd.solve.into();
    let prices = solve(&world, &defs, solve_opts.clone());
    let country = world
        .player_country_tag()
        .context("save has no playable country")?
        .to_string();
    let date = world.game_date.map(|date| date.game_fmt().to_string());
    let state = PlanningState::from_world_with_prices(&world, &country, &prices, &defs)?;
    let goal = vic3_planning::parse(&cmd.goal)?;
    let economy = EconomyContext::new(world, defs, solve_opts);
    let opts = PlanOpts {
        goal: cmd.goal.clone(),
        max_days: cmd.max_days,
        label: cmd.label.clone(),
        allow_pm_changes: cmd.allow_pm_changes,
    };
    let result = vic3_planning::plan_with_economy(
        state,
        goal,
        opts.sim_config(),
        economy,
        cmd.max_days,
        prices.residual,
        prices.limitations,
    )?;
    let record = AnalysisRecord {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: Utc::now().to_rfc3339(),
        label: cmd.label,
        kind: "plan".into(),
        fingerprint: Sha256::digest(&save_bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        date,
        country: Some(country),
        filename: cmd
            .io
            .save
            .file_name()
            .map(|name| name.to_string_lossy().into_owned()),
        opts: serde_json::to_value(&opts)?,
        result: serde_json::to_value(&result)?,
        limitations: result.limitations.clone(),
        parent_id: None,
        blob: None,
    };
    save_record(&record)?;
    emit_plan(&result, cmd.json)
}

fn emit_plan(result: &PlanResult, json: bool) -> Result<()> {
    if json {
        serde_json::to_writer(io::stdout(), result)?;
        writeln!(io::stdout())?;
        return Ok(());
    }
    writeln!(io::stdout(), "total: {} days", result.day_cost)?;
    for step in &result.actions {
        writeln!(io::stdout(), "day {:>4}: {:?}", step.day, step.action)?;
    }
    writeln!(io::stderr(), "warning: {}", result.limitations.join(" "))?;
    Ok(())
}

fn archive_dir() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(root).join("vic3-analyzer"));
    }
    dirs::data_local_dir()
        .map(|root| root.join("vic3-analyzer"))
        .context("could not determine local data directory")
}

fn save_record(record: &AnalysisRecord) -> Result<PathBuf> {
    let dir = archive_dir()?;
    fs::create_dir_all(&dir)
        .with_context(|| format!("creating archive directory {}", dir.display()))?;
    let path = dir.join(format!("{}.json", record.id));
    let bytes = serde_json::to_vec_pretty(record)?;
    fs::write(&path, bytes)
        .with_context(|| format!("writing archive record {}", path.display()))?;
    Ok(path)
}

fn load_records() -> Result<Vec<AnalysisRecord>> {
    let dir = archive_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for entry in fs::read_dir(&dir)
        .with_context(|| format!("reading archive directory {}", dir.display()))?
    {
        let path = entry?.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let record = serde_json::from_slice(&fs::read(&path)?)
            .with_context(|| format!("parsing archive record {}", path.display()))?;
        records.push(record);
    }
    records.sort_by(|a: &AnalysisRecord, b| b.created_at.cmp(&a.created_at));
    Ok(records)
}

fn load_record(id: &str) -> Result<AnalysisRecord> {
    ensure_plain_id(id)?;
    let path = archive_dir()?.join(format!("{id}.json"));
    serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("reading archive record {}", path.display()))?,
    )
    .with_context(|| format!("parsing archive record {}", path.display()))
}

fn run_archive(cmd: ArchiveCli) -> Result<()> {
    match cmd.command {
        ArchiveCommand::List => {
            for record in load_records()? {
                writeln!(
                    io::stdout(),
                    "{}\t{}\t{}\t{}",
                    record.id,
                    record.kind,
                    record.label.as_deref().unwrap_or("-"),
                    record.created_at
                )?;
            }
        }
        ArchiveCommand::Show { id } => {
            let record = load_record(&id)?;
            serde_json::to_writer_pretty(io::stdout(), &record)?;
            writeln!(io::stdout())?;
        }
        ArchiveCommand::Diff { left, right } => {
            let result = compare(&load_record(&left)?, &load_record(&right)?);
            serde_json::to_writer_pretty(io::stdout(), &result)?;
            writeln!(io::stdout())?;
        }
        ArchiveCommand::Export { id, path } => {
            let record = load_record(&id)?;
            fs::write(&path, serde_json::to_vec_pretty(&record)?)
                .with_context(|| format!("writing exported record {}", path.display()))?;
        }
        ArchiveCommand::Import { path } => {
            let record: AnalysisRecord = serde_json::from_slice(
                &fs::read(&path)
                    .with_context(|| format!("reading imported record {}", path.display()))?,
            )
            .with_context(|| format!("parsing imported record {}", path.display()))?;
            ensure_plain_id(&record.id)?;
            save_record(&record)?;
            writeln!(io::stdout(), "{}", record.id)?;
        }
    }
    Ok(())
}

fn ensure_plain_id(id: &str) -> Result<()> {
    anyhow::ensure!(
        Path::new(id).file_name().and_then(|name| name.to_str()) == Some(id),
        "invalid archive id"
    );
    Ok(())
}

fn emit_optimize(result: &OptimizeResult, json: bool) -> Result<()> {
    if json {
        serde_json::to_writer(io::stdout(), result)?;
        writeln!(io::stdout())?;
        return Ok(());
    }
    writeln!(
        io::stdout(),
        "axis {:?}  Δ income {:.4}  productivity {:.4}  SoL {:.4}  residual {:.6}",
        result.axis,
        result.delta.income,
        result.delta.productivity,
        result.delta.sol,
        result.delta.residual
    )?;
    if result.changes.is_empty() {
        writeln!(io::stdout(), "no improving production-method changes")?;
    } else {
        for change in &result.changes {
            writeln!(
                io::stdout(),
                "- {} #{} {:?} → {:?}",
                change.building_type,
                change.building_id,
                change.from,
                change.to
            )?;
        }
    }
    writeln!(io::stderr(), "warning: {}", result.limitations.join(" "))?;
    Ok(())
}

fn emit_alerts(result: &AlertsResult, json: bool) -> Result<()> {
    if json {
        serde_json::to_writer(io::stdout(), result)?;
        writeln!(io::stdout())?;
        return Ok(());
    }
    if result.alerts.is_empty() {
        writeln!(io::stdout(), "no alerts")?;
    } else {
        for alert in &result.alerts {
            writeln!(io::stdout(), "{}  {}", alert.title, alert.summary)?;
        }
    }
    writeln!(io::stderr(), "warning: {}", result.limitations.join(" "))?;
    Ok(())
}

fn emit_gaps(result: &GapsResult, json: bool) -> Result<()> {
    if json {
        serde_json::to_writer(io::stdout(), result)?;
        writeln!(io::stdout())?;
        return Ok(());
    }
    if result.gaps.is_empty() {
        writeln!(io::stdout(), "goal satisfied")?;
    } else {
        for gap in &result.gaps {
            writeln!(io::stdout(), "- {gap:?}")?;
        }
    }
    writeln!(io::stderr(), "warning: {}", result.limitations.join(" "))?;
    Ok(())
}

fn emit(result: &PricesResult, json: bool) -> Result<()> {
    // `solve` already built `state_pops` / need baskets. JSON emits them;
    // the table does not. Skipping that work here would not speed the webapp.
    if json {
        serde_json::to_writer(io::stdout(), result)?;
        writeln!(io::stdout())?;
        return Ok(());
    }
    print_table(result)?;
    writeln!(io::stderr(), "warning: {}", result.limitations.join(" "))?;
    Ok(())
}

fn print_table(result: &PricesResult) -> Result<()> {
    let mut out = io::stdout();
    writeln!(
        out,
        "{:<16} {:>10} {:>10} {:>12} {:>12}",
        "name", "base", "price", "buy", "sell"
    )?;
    for row in &result.goods {
        writeln!(
            out,
            "{:<16} {:>10.4} {:>10.4} {:>12.4} {:>12.4}",
            row.name, row.base, row.price, row.buy, row.sell
        )?;
    }
    writeln!(
        out,
        "residual {}  status {}",
        result.residual, result.status
    )?;
    if result.inputs.is_empty_market() {
        writeln!(
            io::stderr(),
            "warning: no buy or sell orders were reconstructed, so every price above is its base price \
             (pops used: {}, skipped: {}; buildings used: {}, without usable orders: {})",
            result.inputs.pops,
            result.inputs.skipped_pops,
            result.inputs.buildings,
            result.inputs.buildings_without_orders,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn clap_command_tree() {
        Cli::command().debug_assert();
    }
}
