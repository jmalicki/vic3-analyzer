//! CLI: `prices` / `what-if` / `gaps`. clap lives only in this crate.

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use vic3_defs::GameDefs;
use vic3_goals::Atom;
use vic3_load::{empty_tokens, load_path, load_tokens_path, Save};
use vic3_prices::{solve, what_if, PricesResult, SolveOpts, WhatIfOpts, World};
use vic3_world::PlanningState;

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
    let world = World::from_save(&save);
    Ok((save, world, defs))
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
    let state = PlanningState::from_save(&save, country_tag, &prices)?;
    let goal = vic3_goals::parse(&cmd.goal)?;
    let result = GapsResult {
        satisfied: vic3_goals::evaluate(&goal, &state),
        gaps: vic3_goals::gaps(&goal, &state),
        limitations: prices.limitations,
    };
    emit_gaps(&result, cmd.json)
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
