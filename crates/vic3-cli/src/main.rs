//! CLI: prices, what-if, gaps, planning, and the local archive.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use sha2::{Digest, Sha256};
use vic3_defs::GameDefs;
use vic3_goals::Atom;
use vic3_load::{empty_tokens, load_path, load_tokens_path, Save};
use vic3_plan::{compare, AnalysisRecord, PlanOpts, PlanResult};
use vic3_prices::{solve, what_if, PricesResult, SolveOpts, WhatIfOpts, World};
use vic3_sim::{EconomyContext, SimConfig};
use vic3_world::PlanningState;
use vic3save::PdsDate;

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Prices(cmd) => {
            let (_, world, defs) = load_inputs(&cmd.io)?;
            emit(&solve(&world, &defs, cmd.solve.into()), cmd.json)
        }
        Commands::WhatIf(cmd) => {
            let (_, world, defs) = load_inputs(&cmd.io)?;
            emit(
                &what_if(&world, &defs, &cmd.what_if.into(), cmd.solve.into()),
                cmd.json,
            )
        }
        Commands::Gaps(cmd) => run_gaps(cmd),
        Commands::Plan(cmd) => run_plan(cmd),
        Commands::Defs(cmd) => run_defs(cmd),
        Commands::Archive(cmd) => run_archive(cmd),
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
    /// Evaluate a goal and list its currently unsatisfied atoms.
    Gaps(GapsCli),
    /// Find and archive a shortest goal-relevant action sequence.
    Plan(PlanCli),
    /// Build definition blobs for the browser UI.
    Defs(DefsCli),
    /// Browse local analysis records.
    Archive(ArchiveCli),
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
    #[arg(long, env = "VIC3_GAME")]
    game: PathBuf,
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
        }
    }
}

/// Flatten of [`WhatIfOpts`].
#[derive(Debug, Clone, Args)]
struct WhatIfArgs {
    /// Building type id to bump.
    #[arg(long)]
    building: String,
    /// Non-negative extra levels added to matching buildings.
    #[arg(long)]
    extra_levels: u32,
}

impl From<WhatIfArgs> for WhatIfOpts {
    fn from(args: WhatIfArgs) -> Self {
        Self {
            building: args.building,
            extra_levels: args.extra_levels,
        }
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
    gaps: Vec<Atom>,
    limitations: Vec<String>,
}

fn load_inputs(io: &IoArgs) -> Result<(Save, World, GameDefs)> {
    let defs = vic3_defs::load_from_path(&io.game)
        .with_context(|| format!("loading defs from {}", io.game.display()))?;
    let save = if let Some(tokens) = &io.tokens {
        let tokens = load_tokens_path(tokens)
            .with_context(|| format!("loading tokens from {}", tokens.display()))?;
        load_path(&io.save, tokens)
    } else {
        load_path(&io.save, empty_tokens())
    }
    .with_context(|| format!("loading save from {}", io.save.display()))?;
    let world = World::from_save(&save, &defs);
    Ok((save, world, defs))
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

fn run_gaps(cmd: GapsCli) -> Result<()> {
    let (save, world, defs) = load_inputs(&cmd.io)?;
    let prices = solve(&world, &defs, cmd.solve.into());
    let country_tag = save
        .previous_played
        .iter()
        .find_map(|player| player.name.as_deref())
        .or_else(|| {
            save.countries()
                .next()
                .map(|(_, country)| country.definition.as_str())
        })
        .context("save has no playable country")?;
    let state = PlanningState::from_save_with_prices(&save, country_tag, &prices)?;
    let goal = vic3_goals::parse(&cmd.goal)?;
    let result = GapsResult {
        satisfied: vic3_goals::evaluate(&goal, &state),
        gaps: vic3_goals::gaps(&goal, &state),
        limitations: prices.limitations,
    };
    emit_gaps(&result, cmd.json)
}

fn country_tag(save: &Save) -> Result<&str> {
    save.previous_played
        .iter()
        .find_map(|player| player.name.as_deref())
        .or_else(|| {
            save.countries()
                .next()
                .map(|(_, country)| country.definition.as_str())
        })
        .context("save has no playable country")
}

fn run_plan(cmd: PlanCli) -> Result<()> {
    let save_bytes = fs::read(&cmd.io.save)
        .with_context(|| format!("reading save {}", cmd.io.save.display()))?;
    let (save, world, defs) = load_inputs(&cmd.io)?;
    let solve_opts: SolveOpts = cmd.solve.into();
    let prices = solve(&world, &defs, solve_opts);
    let country = country_tag(&save)?;
    let state = PlanningState::from_save_with_prices(&save, country, &prices)?;
    let goal = vic3_goals::parse(&cmd.goal)?;
    let economy = EconomyContext::new(world, defs, solve_opts);
    let result = vic3_plan::plan_with_economy(
        state,
        goal,
        SimConfig::default(),
        economy,
        cmd.max_days,
        prices.residual,
        prices.limitations,
    )?;
    let opts = PlanOpts {
        goal: cmd.goal,
        max_days: cmd.max_days,
        label: cmd.label.clone(),
    };
    let record = AnalysisRecord {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: Utc::now().to_rfc3339(),
        label: cmd.label,
        kind: "plan".into(),
        fingerprint: Sha256::digest(&save_bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        date: save
            .meta_data
            .game_date
            .map(|date| date.game_fmt().to_string()),
        country: Some(country.to_string()),
        filename: cmd
            .io
            .save
            .file_name()
            .map(|name| name.to_string_lossy().into_owned()),
        opts: serde_json::to_value(opts)?,
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
        "id", "base", "price", "buy", "sell"
    )?;
    for row in &result.goods {
        writeln!(
            out,
            "{:<16} {:>10.4} {:>10.4} {:>12.4} {:>12.4}",
            row.id, row.base, row.price, row.buy, row.sell
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
