//! Shared Model Context Protocol (MCP) client detection & configuration engine.
//!
//! Provides platform-filtered discovery and safe, non-destructive configuration
//! merging and removal across desktop AI companions and agents.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Supported AI companion and agent applications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum McpClientKind {
    /// Claude Desktop application (Anthropic).
    ClaudeDesktop,
    /// LM Studio local LLM application.
    LmStudio,
    /// Windows 11 Copilot via On-Device Registry (`odr.exe`).
    WindowsCopilot,
    /// OpenAI Codex CLI and tooling (`config.toml`).
    Codex,
    /// Cursor AI IDE.
    Cursor,
    /// Claude Code CLI agent.
    ClaudeCode,
}

/// Standard server registration key used across all MCP hosts.
pub const MCP_SERVER_NAME: &str = "vic3-analyzer";

impl McpClientKind {
    /// Canonical CLI / JSON identifier string.
    pub const fn id(self) -> &'static str {
        match self {
            Self::ClaudeDesktop => "claude-desktop",
            Self::LmStudio => "lm-studio",
            Self::WindowsCopilot => "windows-copilot",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
            Self::ClaudeCode => "claude-code",
        }
    }

    /// User-facing display title.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::ClaudeDesktop => "Claude Desktop",
            Self::LmStudio => "LM Studio",
            Self::WindowsCopilot => "Windows Copilot (ODR)",
            Self::Codex => "OpenAI Codex",
            Self::Cursor => "Cursor",
            Self::ClaudeCode => "Claude Code",
        }
    }

    /// Match a client from a string ID or alias (case-insensitive).
    pub fn from_id_or_alias(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase().replace('_', "-");
        match s.as_str() {
            "claude-desktop" | "claude" | "claude-app" => Some(Self::ClaudeDesktop),
            "lm-studio" | "lmstudio" => Some(Self::LmStudio),
            "windows-copilot" | "copilot" | "odr" => Some(Self::WindowsCopilot),
            "codex" | "openai" | "openai-codex" => Some(Self::Codex),
            "cursor" => Some(Self::Cursor),
            "claude-code" | "claude-cli" => Some(Self::ClaudeCode),
            _ => None,
        }
    }

    /// Return all supported clients for the current operating system.
    pub fn supported_on_current_os() -> Vec<Self> {
        let mut list = vec![Self::ClaudeDesktop, Self::LmStudio];
        #[cfg(target_os = "windows")]
        {
            list.push(Self::WindowsCopilot);
        }
        list.extend([Self::Codex, Self::Cursor, Self::ClaudeCode]);
        list
    }

    /// Whether this client is supported on the current running OS.
    pub fn is_supported_on_current_os(self) -> bool {
        #[cfg(not(target_os = "windows"))]
        {
            self != Self::WindowsCopilot
        }
        #[cfg(target_os = "windows")]
        {
            true
        }
    }

    /// Resolve the default user-global configuration file path on disk.
    ///
    /// Returns `None` for system-command-driven clients like Windows Copilot (ODR).
    pub fn default_config_path(self) -> Option<PathBuf> {
        match self {
            Self::ClaudeDesktop => {
                #[cfg(target_os = "macos")]
                {
                    dirs::home_dir().map(|h| {
                        h.join("Library/Application Support/Claude/claude_desktop_config.json")
                    })
                }
                #[cfg(target_os = "windows")]
                {
                    dirs::config_dir().map(|c| c.join(r"Claude\claude_desktop_config.json"))
                }
                #[cfg(target_os = "linux")]
                {
                    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
                        let path = PathBuf::from(xdg).join("Claude/claude_desktop_config.json");
                        if path.exists() {
                            return Some(path);
                        }
                    }
                    dirs::home_dir().map(|h| h.join(".config/Claude/claude_desktop_config.json"))
                }
                #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
                {
                    None
                }
            }
            Self::LmStudio => {
                #[cfg(target_os = "windows")]
                {
                    dirs::home_dir().map(|h| h.join(r".lmstudio\mcp.json"))
                }
                #[cfg(not(target_os = "windows"))]
                {
                    dirs::home_dir().map(|h| h.join(".lmstudio/mcp.json"))
                }
            }
            Self::WindowsCopilot => None,
            Self::Codex => {
                #[cfg(target_os = "windows")]
                {
                    dirs::home_dir().map(|h| h.join(r".codex\config.toml"))
                }
                #[cfg(not(target_os = "windows"))]
                {
                    dirs::home_dir().map(|h| h.join(".codex/config.toml"))
                }
            }
            Self::Cursor => {
                #[cfg(target_os = "windows")]
                {
                    dirs::home_dir().map(|h| h.join(r".cursor\mcp.json"))
                }
                #[cfg(not(target_os = "windows"))]
                {
                    dirs::home_dir().map(|h| h.join(".cursor/mcp.json"))
                }
            }
            Self::ClaudeCode => {
                #[cfg(target_os = "windows")]
                {
                    dirs::home_dir().map(|h| h.join(r".claude.json"))
                }
                #[cfg(not(target_os = "windows"))]
                {
                    dirs::home_dir().map(|h| h.join(".claude.json"))
                }
            }
        }
    }

    /// Check if the application appears installed or present on the system.
    pub fn is_detected(self) -> bool {
        if !self.is_supported_on_current_os() {
            return false;
        }

        if let Some(config_path) = self.default_config_path() {
            if config_path.exists() {
                return true;
            }
            if let Some(parent) = config_path.parent() {
                if parent.exists() && parent != Path::new("/") {
                    return true;
                }
            }
        }

        match self {
            Self::ClaudeDesktop => {
                #[cfg(target_os = "macos")]
                {
                    Path::new("/Applications/Claude.app").exists()
                }
                #[cfg(target_os = "windows")]
                {
                    dirs::data_local_dir()
                        .map(|d| d.join(r"Programs\Claude\Claude.exe").exists())
                        .unwrap_or(false)
                }
                #[cfg(target_os = "linux")]
                {
                    which_binary("claude-desktop").is_some()
                }
                #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
                {
                    false
                }
            }
            Self::LmStudio => {
                #[cfg(target_os = "macos")]
                {
                    Path::new("/Applications/LM Studio.app").exists()
                }
                #[cfg(target_os = "windows")]
                {
                    dirs::data_local_dir()
                        .map(|d| d.join(r"Programs\LM Studio\LM Studio.exe").exists())
                        .unwrap_or(false)
                }
                #[cfg(target_os = "linux")]
                {
                    which_binary("lm-studio").is_some()
                }
                #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
                {
                    false
                }
            }
            Self::WindowsCopilot => {
                #[cfg(target_os = "windows")]
                {
                    which_binary("odr.exe").is_some()
                        || Path::new(r"C:\Windows\System32\odr.exe").exists()
                }
                #[cfg(not(target_os = "windows"))]
                {
                    false
                }
            }
            Self::Codex => which_binary("codex").is_some(),
            Self::Cursor => {
                #[cfg(target_os = "macos")]
                {
                    Path::new("/Applications/Cursor.app").exists()
                        || which_binary("cursor").is_some()
                }
                #[cfg(not(target_os = "macos"))]
                {
                    which_binary("cursor").is_some()
                }
            }
            Self::ClaudeCode => which_binary("claude").is_some(),
        }
    }

    /// Check if `vic3-analyzer` is currently configured for this client.
    pub fn is_configured(self) -> bool {
        if !self.is_supported_on_current_os() {
            return false;
        }

        match self {
            Self::WindowsCopilot => {
                #[cfg(target_os = "windows")]
                {
                    check_odr_configured()
                }
                #[cfg(not(target_os = "windows"))]
                {
                    false
                }
            }
            Self::Codex => {
                let Some(path) = self.default_config_path() else {
                    return false;
                };
                if !path.exists() {
                    return false;
                }
                let Ok(contents) = fs::read_to_string(&path) else {
                    return false;
                };
                let Ok(table) = toml::from_str::<toml::Table>(&contents) else {
                    return false;
                };
                table
                    .get("mcp_servers")
                    .and_then(|v| v.as_table())
                    .map(|t| t.contains_key(MCP_SERVER_NAME))
                    .unwrap_or(false)
            }
            _ => {
                let Some(path) = self.default_config_path() else {
                    return false;
                };
                if !path.exists() {
                    return false;
                }
                let Ok(contents) = fs::read_to_string(&path) else {
                    return false;
                };
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) else {
                    return false;
                };
                value
                    .get("mcpServers")
                    .and_then(|v| v.as_object())
                    .map(|m| m.contains_key(MCP_SERVER_NAME))
                    .unwrap_or(false)
            }
        }
    }

    /// Get current full status for this client.
    pub fn status(self) -> McpClientStatus {
        McpClientStatus {
            kind: self,
            id: self.id().to_string(),
            name: self.display_name().to_string(),
            supported: self.is_supported_on_current_os(),
            detected: self.is_detected(),
            configured: self.is_configured(),
            config_path: self.default_config_path(),
        }
    }
}

/// Status summary for an MCP client host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpClientStatus {
    /// Client enum variant.
    pub kind: McpClientKind,
    /// Canonical identifier (e.g. `claude-desktop`).
    pub id: String,
    /// Display title (e.g. `Claude Desktop`).
    pub name: String,
    /// Whether supported on current OS.
    pub supported: bool,
    /// Whether app / config is detected on disk.
    pub detected: bool,
    /// Whether vic3-analyzer server is registered.
    pub configured: bool,
    /// Default config path on disk (if file-based).
    pub config_path: Option<PathBuf>,
}

/// Resolved executable path and default argument vector for MCP launching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedMcpBinary {
    /// Absolute or command path to executable.
    pub command: PathBuf,
    /// Subcommand arguments (e.g. `["mcp"]` or `["mcp", "serve"]`).
    pub args: Vec<String>,
}

/// Resolve the best binary and arguments to invoke the MCP server.
///
/// Order of discovery:
/// 1. `explicit_bin` if provided.
/// 2. `vic3-analyzer` in `PATH`.
/// 3. Installed desktop app bundle location (macOS `/Applications/...`, Windows Program Files).
/// 4. Sibling executable in the current executable's directory.
/// 5. Fallback to `current_exe()`.
pub fn resolve_mcp_binary(explicit_bin: Option<&Path>) -> ResolvedMcpBinary {
    if let Some(explicit) = explicit_bin {
        return ResolvedMcpBinary {
            command: explicit.to_path_buf(),
            args: vec!["mcp".to_string()],
        };
    }

    // 1. Check if vic3-analyzer is in PATH
    #[cfg(target_os = "windows")]
    let analyzer_name = "vic3-analyzer.exe";
    #[cfg(not(target_os = "windows"))]
    let analyzer_name = "vic3-analyzer";

    if let Some(in_path) = which_binary(analyzer_name) {
        return ResolvedMcpBinary {
            command: in_path,
            args: vec!["mcp".to_string()],
        };
    }

    // 2. Check standard OS app bundle / installation paths
    #[cfg(target_os = "macos")]
    {
        let mac_app =
            PathBuf::from("/Applications/Victoria 3 Analyzer.app/Contents/MacOS/vic3-analyzer");
        if mac_app.exists() {
            return ResolvedMcpBinary {
                command: mac_app,
                args: vec!["mcp".to_string()],
            };
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(local_app) = dirs::data_local_dir() {
            let win_path = local_app.join(r"Programs\vic3-analyzer\vic3-analyzer.exe");
            if win_path.exists() {
                return ResolvedMcpBinary {
                    command: win_path,
                    args: vec!["mcp".to_string()],
                };
            }
        }
        let prog_files = PathBuf::from(r"C:\Program Files\Victoria 3 Analyzer\vic3-analyzer.exe");
        if prog_files.exists() {
            return ResolvedMcpBinary {
                command: prog_files,
                args: vec!["mcp".to_string()],
            };
        }
    }

    // 3. Check sibling executable next to current running binary
    if let Ok(current) = std::env::current_exe() {
        if let Some(parent) = current.parent() {
            let sibling = parent.join(analyzer_name);
            if sibling.exists() {
                return ResolvedMcpBinary {
                    command: sibling,
                    args: vec!["mcp".to_string()],
                };
            }
        }
        // 4. Fallback to current executable
        let file_stem = current.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if file_stem == "vic3-cli" {
            return ResolvedMcpBinary {
                command: current,
                args: vec!["mcp".to_string(), "serve".to_string()],
            };
        }
        return ResolvedMcpBinary {
            command: current,
            args: vec!["mcp".to_string()],
        };
    }

    ResolvedMcpBinary {
        command: PathBuf::from(analyzer_name),
        args: vec!["mcp".to_string()],
    }
}

/// Search for a binary name in system `PATH`.
pub fn which_binary(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for split in std::env::split_paths(&path_var) {
        let candidate = split.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Generate the JSON configuration object for `vic3-analyzer`.
pub fn generate_mcp_json_entry(binary: &ResolvedMcpBinary) -> serde_json::Value {
    serde_json::json!({
        "command": binary.command.to_string_lossy().to_string(),
        "args": binary.args
    })
}

/// Safe non-destructive JSON updater: inserts or updates `"vic3-analyzer"` under `"mcpServers"`.
///
/// Preserves other keys and other configured servers.
pub fn update_mcp_json_content(
    original: &str,
    binary: &ResolvedMcpBinary,
) -> Result<String, String> {
    let mut root: serde_json::Value = if original.trim().is_empty() {
        serde_json::json!({ "mcpServers": {} })
    } else {
        serde_json::from_str(original).map_err(|e| format!("invalid JSON: {e}"))?
    };

    if !root.is_object() {
        return Err("root of config file is not a JSON object".to_string());
    }

    let servers = root
        .as_object_mut()
        .unwrap()
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));

    if !servers.is_object() {
        return Err("'mcpServers' property is not a JSON object".to_string());
    }

    servers
        .as_object_mut()
        .unwrap()
        .insert(MCP_SERVER_NAME.to_string(), generate_mcp_json_entry(binary));

    serde_json::to_string_pretty(&root).map_err(|e| format!("serializing JSON: {e}"))
}

/// Remove `"vic3-analyzer"` from a JSON string under `"mcpServers"`.
///
/// Returns `(updated_content, was_modified)`.
pub fn remove_mcp_json_content(original: &str) -> Result<(String, bool), String> {
    if original.trim().is_empty() {
        return Ok((original.to_string(), false));
    }

    let mut root: serde_json::Value =
        serde_json::from_str(original).map_err(|e| format!("invalid JSON: {e}"))?;

    if !root.is_object() {
        return Ok((original.to_string(), false));
    }

    let mut modified = false;
    if let Some(servers) = root.get_mut("mcpServers").and_then(|s| s.as_object_mut()) {
        if servers.remove(MCP_SERVER_NAME).is_some() {
            modified = true;
        }
    }

    if modified {
        let serialized =
            serde_json::to_string_pretty(&root).map_err(|e| format!("serializing JSON: {e}"))?;
        Ok((serialized, true))
    } else {
        Ok((original.to_string(), false))
    }
}

/// Safe non-destructive TOML updater for Codex: inserts `[mcp_servers.vic3-analyzer]`.
pub fn update_codex_toml_content(
    original: &str,
    binary: &ResolvedMcpBinary,
) -> Result<String, String> {
    let mut table: toml::Table = if original.trim().is_empty() {
        toml::Table::new()
    } else {
        toml::from_str(original).map_err(|e| format!("invalid TOML: {e}"))?
    };

    let servers = table
        .entry("mcp_servers")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));

    let servers_table = servers
        .as_table_mut()
        .ok_or_else(|| "'mcp_servers' is not a TOML table".to_string())?;

    let mut server_entry = toml::Table::new();
    server_entry.insert(
        "command".to_string(),
        toml::Value::String(binary.command.to_string_lossy().to_string()),
    );
    let args_array = toml::Value::Array(
        binary
            .args
            .iter()
            .map(|a| toml::Value::String(a.clone()))
            .collect(),
    );
    server_entry.insert("args".to_string(), args_array);

    servers_table.insert(
        MCP_SERVER_NAME.to_string(),
        toml::Value::Table(server_entry),
    );

    toml::to_string_pretty(&table).map_err(|e| format!("serializing TOML: {e}"))
}

/// Remove `vic3-analyzer` from Codex `mcp_servers` table in TOML content.
pub fn remove_codex_toml_content(original: &str) -> Result<(String, bool), String> {
    if original.trim().is_empty() {
        return Ok((original.to_string(), false));
    }

    let mut table: toml::Table =
        toml::from_str(original).map_err(|e| format!("invalid TOML: {e}"))?;

    let mut modified = false;
    if let Some(servers) = table.get_mut("mcp_servers").and_then(|v| v.as_table_mut()) {
        if servers.remove(MCP_SERVER_NAME).is_some() {
            modified = true;
        }
    }

    if modified {
        let serialized =
            toml::to_string_pretty(&table).map_err(|e| format!("serializing TOML: {e}"))?;
        Ok((serialized, true))
    } else {
        Ok((original.to_string(), false))
    }
}

/// Install configuration for a specific client.
///
/// If `dry_run` is true, returns the serialized file content without writing.
pub fn install_client_config(
    client: McpClientKind,
    binary: &ResolvedMcpBinary,
    dry_run: bool,
) -> Result<String, String> {
    if !client.is_supported_on_current_os() {
        return Err(format!(
            "{} is not supported on this operating system",
            client.display_name()
        ));
    }

    match client {
        McpClientKind::WindowsCopilot => {
            #[cfg(target_os = "windows")]
            {
                let cmd_str = format!(
                    "odr.exe mcp add --name {} --command \"{}\" --args \"{}\"",
                    MCP_SERVER_NAME,
                    binary.command.display(),
                    binary.args.join(" ")
                );
                if dry_run {
                    return Ok(cmd_str);
                }
                execute_odr_add(binary)?;
                Ok(cmd_str)
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = binary;
                let _ = dry_run;
                Err("Windows Copilot (ODR) is only supported on Windows".to_string())
            }
        }
        McpClientKind::Codex => {
            let path = client
                .default_config_path()
                .ok_or_else(|| "cannot determine config path".to_string())?;
            let existing = if path.exists() {
                fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))?
            } else {
                String::new()
            };
            let updated = update_codex_toml_content(&existing, binary)?;
            if !dry_run {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("creating dir {}: {e}", parent.display()))?;
                }
                fs::write(&path, &updated)
                    .map_err(|e| format!("writing {}: {e}", path.display()))?;
            }
            Ok(updated)
        }
        _ => {
            let path = client
                .default_config_path()
                .ok_or_else(|| "cannot determine config path".to_string())?;
            let existing = if path.exists() {
                fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))?
            } else {
                String::new()
            };
            let updated = update_mcp_json_content(&existing, binary)?;
            if !dry_run {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("creating dir {}: {e}", parent.display()))?;
                }
                fs::write(&path, &updated)
                    .map_err(|e| format!("writing {}: {e}", path.display()))?;
            }
            Ok(updated)
        }
    }
}

/// Uninstall configuration for a specific client.
///
/// Returns `true` if configuration was removed, `false` if not found.
pub fn uninstall_client_config(client: McpClientKind, dry_run: bool) -> Result<bool, String> {
    if !client.is_supported_on_current_os() {
        return Ok(false);
    }

    match client {
        McpClientKind::WindowsCopilot => {
            #[cfg(target_os = "windows")]
            {
                if dry_run {
                    return Ok(true);
                }
                execute_odr_remove()
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = dry_run;
                Ok(false)
            }
        }
        McpClientKind::Codex => {
            let Some(path) = client.default_config_path() else {
                return Ok(false);
            };
            if !path.exists() {
                return Ok(false);
            }
            let existing = fs::read_to_string(&path)
                .map_err(|e| format!("reading {}: {e}", path.display()))?;
            let (updated, modified) = remove_codex_toml_content(&existing)?;
            if modified && !dry_run {
                fs::write(&path, updated)
                    .map_err(|e| format!("writing {}: {e}", path.display()))?;
            }
            Ok(modified)
        }
        _ => {
            let Some(path) = client.default_config_path() else {
                return Ok(false);
            };
            if !path.exists() {
                return Ok(false);
            }
            let existing = fs::read_to_string(&path)
                .map_err(|e| format!("reading {}: {e}", path.display()))?;
            let (updated, modified) = remove_mcp_json_content(&existing)?;
            if modified && !dry_run {
                fs::write(&path, updated)
                    .map_err(|e| format!("writing {}: {e}", path.display()))?;
            }
            Ok(modified)
        }
    }
}

/// Format the raw snippet for manual user pasting.
pub fn format_client_snippet(
    client: McpClientKind,
    binary: &ResolvedMcpBinary,
) -> Result<String, String> {
    match client {
        McpClientKind::WindowsCopilot => Ok(format!(
            "odr.exe mcp add --name {} --command \"{}\" --args \"{}\"",
            MCP_SERVER_NAME,
            binary.command.display(),
            binary.args.join(" ")
        )),
        McpClientKind::Codex => update_codex_toml_content("", binary),
        _ => update_mcp_json_content("", binary),
    }
}

#[cfg(target_os = "windows")]
fn check_odr_configured() -> bool {
    let output = std::process::Command::new("odr.exe")
        .args(["mcp", "list"])
        .output();
    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            text.contains(MCP_SERVER_NAME)
        }
        Err(_) => false,
    }
}

#[cfg(target_os = "windows")]
fn execute_odr_add(binary: &ResolvedMcpBinary) -> Result<(), String> {
    let mut cmd = std::process::Command::new("odr.exe");
    cmd.arg("mcp")
        .arg("add")
        .arg("--name")
        .arg(MCP_SERVER_NAME)
        .arg("--command")
        .arg(binary.command.to_string_lossy().to_string());
    if !binary.args.is_empty() {
        cmd.arg("--args").arg(binary.args.join(" "));
    }
    let status = cmd
        .status()
        .map_err(|e| format!("failed to execute odr.exe: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("odr.exe exited with code {:?}", status.code()))
    }
}

#[cfg(target_os = "windows")]
fn execute_odr_remove() -> Result<bool, String> {
    let status = std::process::Command::new("odr.exe")
        .args(["mcp", "remove", "--name", MCP_SERVER_NAME])
        .status()
        .map_err(|e| format!("failed to execute odr.exe: {e}"))?;
    Ok(status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_binary() -> ResolvedMcpBinary {
        ResolvedMcpBinary {
            command: PathBuf::from("/usr/local/bin/vic3-analyzer"),
            args: vec!["mcp".to_string()],
        }
    }

    #[test]
    fn test_mcp_client_kind_round_trip() {
        assert_eq!(
            McpClientKind::from_id_or_alias("claude-desktop"),
            Some(McpClientKind::ClaudeDesktop)
        );
        assert_eq!(
            McpClientKind::from_id_or_alias("claude"),
            Some(McpClientKind::ClaudeDesktop)
        );
        assert_eq!(
            McpClientKind::from_id_or_alias("lm-studio"),
            Some(McpClientKind::LmStudio)
        );
        assert_eq!(
            McpClientKind::from_id_or_alias("lmstudio"),
            Some(McpClientKind::LmStudio)
        );
        assert_eq!(
            McpClientKind::from_id_or_alias("codex"),
            Some(McpClientKind::Codex)
        );
        assert_eq!(
            McpClientKind::from_id_or_alias("openai"),
            Some(McpClientKind::Codex)
        );
        assert_eq!(
            McpClientKind::from_id_or_alias("cursor"),
            Some(McpClientKind::Cursor)
        );
        assert_eq!(
            McpClientKind::from_id_or_alias("claude-code"),
            Some(McpClientKind::ClaudeCode)
        );
    }

    #[test]
    fn test_json_update_new_file() {
        let bin = mock_binary();
        let result = update_mcp_json_content("", &bin).unwrap();
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        let server = &value["mcpServers"]["vic3-analyzer"];
        assert_eq!(server["command"], "/usr/local/bin/vic3-analyzer");
        assert_eq!(server["args"][0], "mcp");
    }

    #[test]
    fn test_json_update_preserves_other_servers_and_keys() {
        let bin = mock_binary();
        let existing = r#"{
  "theme": "dark",
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem"]
    }
  }
}"#;
        let result = update_mcp_json_content(existing, &bin).unwrap();
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["theme"], "dark");
        assert_eq!(value["mcpServers"]["filesystem"]["command"], "npx");
        assert_eq!(
            value["mcpServers"]["vic3-analyzer"]["command"],
            "/usr/local/bin/vic3-analyzer"
        );
    }

    #[test]
    fn test_json_remove_server() {
        let existing = r#"{
  "mcpServers": {
    "vic3-analyzer": { "command": "foo" },
    "other": { "command": "bar" }
  }
}"#;
        let (result, modified) = remove_mcp_json_content(existing).unwrap();
        assert!(modified);
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(value["mcpServers"].get("vic3-analyzer").is_none());
        assert!(value["mcpServers"].get("other").is_some());

        // Second removal returns false
        let (_, modified2) = remove_mcp_json_content(&result).unwrap();
        assert!(!modified2);
    }

    #[test]
    fn test_codex_toml_update_and_remove() {
        let bin = mock_binary();
        let result = update_codex_toml_content("", &bin).unwrap();
        let table: toml::Table = toml::from_str(&result).unwrap();
        let server = &table["mcp_servers"]["vic3-analyzer"];
        assert_eq!(
            server["command"].as_str().unwrap(),
            "/usr/local/bin/vic3-analyzer"
        );

        let (removed, modified) = remove_codex_toml_content(&result).unwrap();
        assert!(modified);
        let table2: toml::Table = toml::from_str(&removed).unwrap();
        assert!(!table2["mcp_servers"]
            .as_table()
            .unwrap()
            .contains_key("vic3-analyzer"));
    }

    #[test]
    fn test_snippet_generation() {
        let bin = mock_binary();
        let claude_snippet = format_client_snippet(McpClientKind::ClaudeDesktop, &bin).unwrap();
        assert!(claude_snippet.contains("\"vic3-analyzer\""));
        assert!(claude_snippet.contains("\"/usr/local/bin/vic3-analyzer\""));

        let codex_snippet = format_client_snippet(McpClientKind::Codex, &bin).unwrap();
        assert!(codex_snippet.contains("[mcp_servers.vic3-analyzer]"));
    }
}
