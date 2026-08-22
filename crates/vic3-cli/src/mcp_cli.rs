//! CLI handler for Model Context Protocol (MCP) server management.
//!
//! Provides `vic3-cli mcp install`, `status`, `uninstall`, `show-config`, and `serve`.

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use clap::{Args, Subcommand, ValueEnum};
use vic3_catalog::{
    format_client_snippet, install_client_config, resolve_mcp_binary, uninstall_client_config,
    McpClientKind, McpClientStatus, ResolvedMcpBinary,
};

#[derive(Debug, Args)]
pub struct McpCli {
    #[command(subcommand)]
    pub command: McpCommand,
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// Automatically configure the MCP server in desktop AI applications.
    Install(InstallArgs),
    /// Remove the MCP server registration from desktop AI applications.
    Uninstall(UninstallArgs),
    /// Inspect MCP integration status for supported desktop AI applications.
    Status(StatusArgs),
    /// Emit configuration snippet (JSON/TOML) for manual copying.
    ShowConfig(ShowConfigArgs),
    /// Run the headless MCP stdio server directly from vic3-cli.
    Serve,
}

/// Target client selection for CLI flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ClientArg {
    /// Claude Desktop app
    ClaudeDesktop,
    /// LM Studio local LLM app
    LmStudio,
    /// Windows 11 Copilot (ODR)
    WindowsCopilot,
    /// OpenAI Codex CLI
    Codex,
    /// Cursor AI IDE
    Cursor,
    /// Claude Code CLI
    ClaudeCode,
    /// All supported clients on the current operating system
    All,
}

impl ClientArg {
    pub fn to_mcp_client_kind(self) -> Option<McpClientKind> {
        match self {
            Self::ClaudeDesktop => Some(McpClientKind::ClaudeDesktop),
            Self::LmStudio => Some(McpClientKind::LmStudio),
            Self::WindowsCopilot => Some(McpClientKind::WindowsCopilot),
            Self::Codex => Some(McpClientKind::Codex),
            Self::Cursor => Some(McpClientKind::Cursor),
            Self::ClaudeCode => Some(McpClientKind::ClaudeCode),
            Self::All => None,
        }
    }
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    /// Specific AI client to configure (defaults to prompting or auto-detecting all).
    #[arg(long, short, value_enum)]
    pub client: Option<ClientArg>,

    /// Override the binary executable path written into client configs.
    #[arg(long)]
    pub bin: Option<PathBuf>,

    /// Override the arguments passed to the executable (e.g. --args mcp).
    #[arg(long, num_args = 1..)]
    pub args: Option<Vec<String>>,

    /// Preview configuration changes without writing to disk.
    #[arg(long)]
    pub dry_run: bool,

    /// Non-interactive mode: automatically install to all detected applications without prompting.
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Force interactive checkbox prompt even in non-TTY environments.
    #[arg(long, short = 'i')]
    pub interactive: bool,
}

#[derive(Debug, Args)]
pub struct UninstallArgs {
    /// Specific AI client to remove configuration from.
    #[arg(long, short, value_enum)]
    pub client: Option<ClientArg>,

    /// Uninstall from all supported AI clients on this system.
    #[arg(long)]
    pub all: bool,

    /// Preview uninstallation without modifying files.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Output machine-readable JSON array of client status objects.
    #[arg(long)]
    pub json: bool,

    /// Filter status to a specific AI client.
    #[arg(long, short, value_enum)]
    pub client: Option<ClientArg>,
}

#[derive(Debug, Args)]
pub struct ShowConfigArgs {
    /// Target AI client format to display.
    #[arg(long, short, value_enum, default_value = "claude-desktop")]
    pub client: ClientArg,

    /// Override the binary executable path in the snippet.
    #[arg(long)]
    pub bin: Option<PathBuf>,

    /// Override the arguments in the snippet.
    #[arg(long, num_args = 1..)]
    pub args: Option<Vec<String>>,
}

/// Dispatcher for `vic3-cli mcp` commands.
pub fn run_mcp(cmd: McpCli) -> Result<()> {
    match cmd.command {
        McpCommand::Install(args) => run_install(args),
        McpCommand::Uninstall(args) => run_uninstall(args),
        McpCommand::Status(args) => run_status(args),
        McpCommand::ShowConfig(args) => run_show_config(args),
        McpCommand::Serve => run_serve(),
    }
}

fn build_resolved_binary(bin: Option<&Path>, custom_args: Option<&[String]>) -> ResolvedMcpBinary {
    let mut resolved = resolve_mcp_binary(bin);
    if let Some(args) = custom_args {
        resolved.args = args.to_vec();
    }
    resolved
}

fn run_install(args: InstallArgs) -> Result<()> {
    let binary = build_resolved_binary(args.bin.as_deref(), args.args.as_deref());
    let supported = McpClientKind::supported_on_current_os();

    // 1. Explicit single client or all requested via flag
    if let Some(client_arg) = args.client {
        let targets = match client_arg.to_mcp_client_kind() {
            Some(kind) => vec![kind],
            None => supported,
        };
        for target in targets {
            install_single(target, &binary, args.dry_run)?;
        }
        return Ok(());
    }

    // 2. Determine target selection (interactive prompt vs auto-detect)
    let is_interactive = (io::stdin().is_terminal() && !args.yes) || args.interactive;
    let selected_targets = if is_interactive {
        prompt_client_selection(&supported)?
    } else {
        // Auto-select detected clients (or all supported if none detected)
        let detected: Vec<_> = supported
            .iter()
            .copied()
            .filter(|c| c.is_detected())
            .collect();
        if detected.is_empty() {
            writeln!(
                io::stderr(),
                "No desktop AI applications detected. Configuring all supported clients for this OS..."
            )?;
            supported
        } else {
            detected
        }
    };

    if selected_targets.is_empty() {
        writeln!(io::stdout(), "No applications selected. Nothing installed.")?;
        return Ok(());
    }

    for target in selected_targets {
        install_single(target, &binary, args.dry_run)?;
    }

    Ok(())
}

fn prompt_client_selection(supported: &[McpClientKind]) -> Result<Vec<McpClientKind>> {
    let mut statuses: Vec<(McpClientKind, bool)> = supported
        .iter()
        .map(|&client| (client, client.is_detected()))
        .collect();

    writeln!(
        io::stdout(),
        "\nSelect AI applications to configure with Victoria 3 MCP server:\n"
    )?;

    loop {
        for (i, (client, checked)) in statuses.iter().enumerate() {
            let check_mark = if *checked { "[X]" } else { "[ ]" };
            let path_hint = client
                .default_config_path()
                .map(|p| format!("({})", p.display()))
                .unwrap_or_else(|| "(Windows ODR command)".to_string());
            let detected_tag = if client.is_detected() {
                " (detected)"
            } else {
                ""
            };
            writeln!(
                io::stdout(),
                "  {check_mark} [{}] {} {detected_tag}\n        {}",
                i + 1,
                client.display_name(),
                path_hint
            )?;
        }

        writeln!(
            io::stdout(),
            "\nEnter numbers to toggle (e.g. '1 2'), 'a' for all, 'n' for none, or press Enter to proceed:"
        )?;
        print!("> ");
        io::stdout().flush()?;

        let mut input = String::new();
        let bytes_read = io::stdin().lock().read_line(&mut input)?;
        if bytes_read == 0 {
            // EOF (piped / non-interactive fallback)
            break;
        }

        let trimmed = input.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            break;
        }

        if trimmed == "a" || trimmed == "all" {
            for item in &mut statuses {
                item.1 = true;
            }
        } else if trimmed == "n" || trimmed == "none" {
            for item in &mut statuses {
                item.1 = false;
            }
        } else {
            for token in trimmed.split_whitespace() {
                if let Ok(idx) = token.parse::<usize>() {
                    if idx >= 1 && idx <= statuses.len() {
                        statuses[idx - 1].1 = !statuses[idx - 1].1;
                    }
                }
            }
        }
        writeln!(io::stdout())?;
    }

    Ok(statuses
        .into_iter()
        .filter_map(|(client, checked)| if checked { Some(client) } else { None })
        .collect())
}

fn install_single(client: McpClientKind, binary: &ResolvedMcpBinary, dry_run: bool) -> Result<()> {
    match install_client_config(client, binary, dry_run) {
        Ok(result) => {
            if dry_run {
                writeln!(
                    io::stdout(),
                    "[DRY-RUN] Would configure {}:\n{}\n",
                    client.display_name(),
                    result
                )?;
            } else {
                let location = client
                    .default_config_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "Windows On-Device Registry".to_string());
                writeln!(
                    io::stdout(),
                    "✓ Configured {} ({})",
                    client.display_name(),
                    location
                )?;
            }
            Ok(())
        }
        Err(err) => {
            writeln!(
                io::stderr(),
                "✗ Failed to configure {}: {err}",
                client.display_name()
            )?;
            bail!("Configuration failed for {}", client.display_name())
        }
    }
}

fn run_uninstall(args: UninstallArgs) -> Result<()> {
    let supported = McpClientKind::supported_on_current_os();
    let targets = if args.all {
        supported
    } else if let Some(client_arg) = args.client {
        match client_arg.to_mcp_client_kind() {
            Some(kind) => vec![kind],
            None => supported,
        }
    } else {
        // Default: uninstall from any configured client
        supported
            .into_iter()
            .filter(|c| c.is_configured())
            .collect()
    };

    if targets.is_empty() {
        writeln!(
            io::stdout(),
            "No configured AI applications found to remove."
        )?;
        return Ok(());
    }

    for target in targets {
        match uninstall_client_config(target, args.dry_run) {
            Ok(modified) => {
                if args.dry_run {
                    writeln!(
                        io::stdout(),
                        "[DRY-RUN] Would remove vic3-analyzer from {}",
                        target.display_name()
                    )?;
                } else if modified {
                    writeln!(
                        io::stdout(),
                        "✓ Removed vic3-analyzer configuration from {}",
                        target.display_name()
                    )?;
                } else {
                    writeln!(
                        io::stdout(),
                        "- vic3-analyzer was not configured in {}",
                        target.display_name()
                    )?;
                }
            }
            Err(err) => {
                writeln!(
                    io::stderr(),
                    "✗ Failed to uninstall from {}: {err}",
                    target.display_name()
                )?;
            }
        }
    }

    Ok(())
}

fn run_status(args: StatusArgs) -> Result<()> {
    let mut clients = McpClientKind::supported_on_current_os();
    if let Some(target) = args.client.and_then(|c| c.to_mcp_client_kind()) {
        clients.retain(|&c| c == target);
    }

    let statuses: Vec<McpClientStatus> = clients.iter().map(|c| c.status()).collect();

    if args.json {
        serde_json::to_writer_pretty(io::stdout(), &statuses)?;
        writeln!(io::stdout())?;
        return Ok(());
    }

    let mut out = io::stdout();
    writeln!(
        out,
        "{:<24} {:<12} {:<12} {:<14} CONFIG PATH",
        "APPLICATION", "SUPPORTED", "DETECTED", "CONFIGURED"
    )?;
    writeln!(out, "{}", "-".repeat(80))?;

    for s in &statuses {
        let path_str = s
            .config_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(odr.exe)".to_string());
        writeln!(
            out,
            "{:<24} {:<12} {:<12} {:<14} {}",
            s.name,
            if s.supported { "yes" } else { "no" },
            if s.detected { "yes" } else { "no" },
            if s.configured { "yes" } else { "no" },
            path_str
        )?;
    }

    Ok(())
}

fn run_show_config(args: ShowConfigArgs) -> Result<()> {
    let binary = build_resolved_binary(args.bin.as_deref(), args.args.as_deref());
    let kind = args
        .client
        .to_mcp_client_kind()
        .unwrap_or(McpClientKind::ClaudeDesktop);

    let snippet = format_client_snippet(kind, &binary).map_err(|e| anyhow::anyhow!("{e}"))?;
    writeln!(io::stdout(), "{snippet}")?;
    Ok(())
}

fn run_serve() -> Result<()> {
    let exit_code = vic3_mcp::run();
    if exit_code == std::process::ExitCode::SUCCESS {
        Ok(())
    } else {
        bail!("MCP server exited with error");
    }
}
