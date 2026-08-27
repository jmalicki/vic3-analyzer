//! rmcp [`ServerHandler`]: tools, resources, prompts, completions.
//!
//! Tool argument shapes are derived via schemars (`docs/mcp.md`). Logging stays
//! on stderr through tracing; stdout is protocol-only.
//!
//! Shared engine: every tool goes through [`McpRuntime`] → [`vic3_sql::SqlEngine`]
//! (same dialect as Tauri Advanced Query).

use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use rmcp::handler::server::router::prompt::PromptRouter;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::{
    prompt, prompt_handler, prompt_router, schemars, tool, tool_handler, tool_router, RoleServer,
    ServerHandler, ServiceExt,
};
use serde::Deserialize;
use serde_json::json;
use vic3_catalog::SaveLocation;
use vic3_prices::WorldDelta;
use vic3_sql::{schema_catalog_json, UseSaveRequest};

use crate::error::{sql_to_tool_result, tool_err, tool_ok_json, tool_ok_text};
use crate::format::{batches_to_csv, batches_to_json};
use crate::runtime::{world_delta_from_sugar, McpRuntime};

/// Docs embedded for `vic3://docs/*` (paths relative to this crate).
const DOC_SQL: &str = include_str!("../../../docs/sql.md");
const DOC_MCP: &str = include_str!("../../../docs/mcp.md");

/// Agent flow excerpt kept short for `vic3://docs/flow`.
const FLOW_MARKDOWN: &str = r#"# Vic3 Analyzer MCP flow

1. Discover saves: tool `query` on `saves`, or read resource `vic3://saves`.
2. Bind session: tool `use_save` with `{ "name": "autosave" }` or `{ "selector": "latest_autosave" }`.
3. Prefer tool `campaign_brief` for a compact session overview, then query / `preview_delta` as needed (`alerts()` is player-scoped; use `alerts('all')` for the full save).

Rules:
- Filename **stubs** only (no filesystem paths).
- SQL is **read-only** (`SELECT` / `WITH` / `EXPLAIN`).
- Call `use_save` before `campaign_brief` / unqualified fact tables; do not use `SELECT set_active_save(...)`.
"#;

/// rmcp [`ServerHandler`] over a process-local [`McpRuntime`].
///
/// Tools/resources/prompts are the agent contract (`docs/mcp.md`). Resource URIs
/// are fixed: `vic3://schema|saves|session|docs/{flow,sql,mcp}`.
#[derive(Clone)]
pub struct Vic3McpServer {
    runtime: Arc<McpRuntime>,
    tool_router: ToolRouter<Self>,
    prompt_router: PromptRouter<Self>,
}

impl Vic3McpServer {
    /// Build routers over an already-opened [`McpRuntime`] (tests or custom serve).
    pub fn new(runtime: McpRuntime) -> Self {
        Self {
            runtime: Arc::new(runtime),
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
        }
    }

    /// Serve over stdin/stdout until the client disconnects.
    ///
    /// # Errors
    ///
    /// Transport / protocol errors from rmcp after the server has started.
    pub async fn serve_stdio(
        runtime: McpRuntime,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let server = Self::new(runtime);
        let service = server.serve(rmcp::transport::stdio()).await?;
        service.waiting().await?;
        Ok(())
    }

    /// Tool list/schemas for contract tests (no transport).
    pub fn tool_router_ref(&self) -> &ToolRouter<Self> {
        &self.tool_router
    }

    /// Expose the prompt router for schema tests (no transport).
    pub fn prompt_router_ref(&self) -> &PromptRouter<Self> {
        &self.prompt_router
    }

    /// Shared config + SQL session backing this server.
    pub fn runtime(&self) -> &McpRuntime {
        &self.runtime
    }
}

// --- Tool argument types (JSON Schema for clients) -------------------------

/// Args for tool `query`: one read-only SQL statement + optional result format.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryArgs {
    /// Single read-only SQL statement (`docs/sql.md`).
    pub sql: String,
    /// `json` (default) or `csv`.
    #[serde(default)]
    pub format: Option<String>,
}

/// Args for tool `use_save`: bind session by stub or selector (not SQL).
///
/// Exactly one of `name` / `selector`; `location` / `mtime` disambiguate stubs.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UseSaveArgs {
    /// Filename stub (`autosave` or `autosave.v3`). Exactly one of name/selector.
    #[serde(default)]
    pub name: Option<String>,
    /// `latest` | `latest_autosave` | `latest_named`.
    #[serde(default)]
    pub selector: Option<String>,
    /// `local` | `steam_cloud` — disambiguate stub.
    #[serde(default)]
    pub location: Option<String>,
    /// ISO-8601 timestamp for further disambiguation.
    #[serde(default)]
    pub mtime: Option<String>,
}

/// Args for tool `refresh_catalog`: empty object so clients can send `{}`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RefreshCatalogArgs {
    // No fields — schemars still emits an object schema.
}

/// Args for tool `campaign_brief`: empty object so clients can send `{}`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CampaignBriefArgs {
    // No fields — schemars still emits an object schema.
}

/// Args for tool `explain`: SQL to wrap as `EXPLAIN …` when needed.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExplainArgs {
    /// SQL to explain (wrapped as `EXPLAIN …` if needed).
    pub sql: String,
}

/// Args for tool `preview_delta`: sugar extra-levels and/or a full [`WorldDelta`].
///
/// Sugar (`building` / `extra_levels` / optional `building_id` / `state_id`) and
/// `delta` are mutually exclusive. Requires a bound session (`use_save`).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewDeltaArgs {
    /// Building type id (e.g. `building_rye_farm`). Sugar with `extra_levels`.
    #[serde(default)]
    pub building: Option<String>,
    /// Extra levels to add (sugar).
    #[serde(default)]
    pub extra_levels: Option<u32>,
    /// Target a single building instance (sugar; wins over `building`).
    #[serde(default)]
    pub building_id: Option<u32>,
    /// Restrict sugar `building` matches to this state id.
    #[serde(default)]
    pub state_id: Option<u32>,
    /// Full [`WorldDelta`] JSON (extra_levels / production_methods / subsidize).
    #[serde(default)]
    pub delta: Option<WorldDelta>,
}

#[tool_router]
impl Vic3McpServer {
    /// Run one read-only SQL statement against the shared DataFusion engine.
    #[tool(
        description = "Run one read-only SQL statement (vic3-sql). Prefer stubs via use_save first; format is json (default) or csv."
    )]
    async fn query(
        &self,
        Parameters(args): Parameters<QueryArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let format = args.format.as_deref().unwrap_or("json");
        match self.runtime.query(&args.sql).await {
            Ok(batches) => match format {
                "json" => match batches_to_json(&batches) {
                    Ok(v) => Ok(tool_ok_json(&v)),
                    Err(e) => Ok(tool_err(e.to_string())),
                },
                "csv" => match batches_to_csv(&batches) {
                    Ok(text) => Ok(tool_ok_text(text)),
                    Err(e) => Ok(tool_err(e.to_string())),
                },
                other => Ok(tool_err(format!(
                    "unknown format {other:?}; use \"json\" or \"csv\""
                ))),
            },
            Err(e) => Ok(sql_to_tool_result(e)),
        }
    }

    /// Bind the analysis session by stub or selector (host API, not SQL).
    #[tool(
        description = "Bind the analysis session by filename stub or selector (latest|latest_autosave|latest_named). Loads/solves the save. Disambiguate with location/mtime when needed."
    )]
    async fn use_save(
        &self,
        Parameters(args): Parameters<UseSaveArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = match build_use_save_request(&args) {
            Ok(r) => r,
            Err(msg) => return Ok(tool_err(msg)),
        };

        // Best-effort progress; clients without progress support ignore this.
        let _ = ctx
            .peer
            .notify_progress(
                ProgressNotificationParam::new(ProgressToken(NumberOrString::Number(1)), 0.0)
                    .with_total(1.0)
                    .with_message("loading save…"),
            )
            .await;

        match self.runtime.use_save(req).await {
            Ok(result) => {
                let body = json!({
                    "name": result.name,
                    "kind": result.kind,
                    "in_game_date": result.in_game_date,
                    "country": result.country,
                    "loaded": result.loaded,
                });
                Ok(tool_ok_json(&body))
            }
            Err(e) => Ok(sql_to_tool_result(e)),
        }
    }

    /// Rescan allowlisted save directories from shared app config.
    #[tool(description = "Rescan allowlisted save directories and refresh the saves catalog.")]
    async fn refresh_catalog(
        &self,
        Parameters(_args): Parameters<RefreshCatalogArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        match self.runtime.refresh_catalog().await {
            Ok(count) => Ok(tool_ok_json(&json!({ "count": count }))),
            Err(e) => Ok(sql_to_tool_result(e)),
        }
    }

    /// Compact campaign summary for the bound save (shortages + alert kinds).
    #[tool(
        description = "After use_save: compact JSON brief (session, player_tag, top domestic goods shortages, state×good hotspots, player-scoped alert kind histogram). Requires a bound save."
    )]
    async fn campaign_brief(
        &self,
        Parameters(_args): Parameters<CampaignBriefArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        match self.runtime.campaign_brief().await {
            Ok(body) => Ok(tool_ok_json(&body)),
            Err(e) => Ok(sql_to_tool_result(e)),
        }
    }

    /// Return a DataFusion EXPLAIN plan for debugging agent SQL.
    #[tool(description = "Return DataFusion EXPLAIN plan text for a SQL statement.")]
    async fn explain(
        &self,
        Parameters(args): Parameters<ExplainArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let sql = normalize_explain_sql(&args.sql);
        match self.runtime.query(&sql).await {
            Ok(batches) => match batches_to_csv(&batches) {
                Ok(text) => Ok(tool_ok_text(text)),
                Err(e) => Ok(tool_err(e.to_string())),
            },
            Err(e) => Ok(sql_to_tool_result(e)),
        }
    }

    /// Warm-started WorldDelta / what-if preview on the bound session (compact JSON).
    #[tool(
        description = "Preview a WorldDelta (or sugar building + extra_levels) on the bound save. Returns compact before/after goods prices & shortages — not a full PricesResult. Requires use_save first. Sugar and delta are mutually exclusive."
    )]
    async fn preview_delta(
        &self,
        Parameters(args): Parameters<PreviewDeltaArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        match resolve_preview_delta(&args, &self.runtime).await {
            Ok(body) => Ok(tool_ok_json(&body)),
            Err(msg) => Ok(tool_err(msg)),
        }
    }
}

#[prompt_router]
impl Vic3McpServer {
    #[prompt(
        name = "investigate_shortages",
        description = "Guide: use_save → campaign_brief → optional shortage SQL"
    )]
    async fn investigate_shortages(&self) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            "Investigate goods shortages in the current Vic3 campaign.\n\
             1. Call use_save with selector latest_autosave (or latest).\n\
             2. Call campaign_brief for session meta, top domestic shortages, hotspots, and alert kinds.\n\
             3. If you need more detail, query e.g.\n\
             SELECT s.label, g.good, g.shortage, g.price\n\
             FROM states s JOIN goods_by_state g USING (state_id)\n\
             WHERE g.shortage > 0\n\
             ORDER BY g.shortage DESC LIMIT 20.\n\
             (Short names are already player-scoped; use world_* for save-wide.)\n\
             Optionally also check market-wide SELECT * FROM goods WHERE shortage > 0.\n\
             Rules: stubs not paths; read-only SQL; use_save before campaign_brief / fact tables.",
        )]
    }

    #[prompt(
        name = "compare_latest_autosave",
        description = "Catalog + bind latest autosave + summary queries"
    )]
    async fn compare_latest_autosave(&self) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            "Compare the latest autosave to the catalog context.\n\
             1. refresh_catalog, then query the saves table.\n\
             2. use_save with selector latest_autosave.\n\
             3. Summarize: countries, goods shortages, and gaps('research(tech=nitroglycerin)') if relevant.\n\
             Rules: stubs not paths; read-only SQL.",
        )]
    }

    #[prompt(
        name = "military_readiness",
        description = "Military / formations readiness queries when available"
    )]
    async fn military_readiness(&self) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            "Assess military readiness for the loaded campaign.\n\
             1. use_save (latest or a named stub).\n\
             2. Query available military-related tables/UDFs; if a table is missing, say so and fall back to countries / buildings.\n\
             Rules: stubs not paths; read-only SQL; use_save first.",
        )]
    }

    #[prompt(
        name = "what_is_loaded",
        description = "Read vic3://session and simple catalog/fact counts"
    )]
    async fn what_is_loaded(&self) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            "Report what is loaded in this MCP session.\n\
             1. Read resource vic3://session.\n\
             2. Query SELECT COUNT(*) AS n FROM saves.\n\
             3. If a save is loaded, show tag/date via active session and a small countries sample.\n\
             Rules: stubs not paths; read-only SQL.",
        )]
    }

    #[prompt(
        name = "plan_research",
        description = "plan('research(tech=…)') pattern for A* research plans"
    )]
    async fn plan_research(&self) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            "Produce a research plan with the plan() TVF.\n\
             1. use_save with selector latest (or a stub).\n\
             2. Run: SELECT step, day, action, detail FROM plan('research(tech=nitroglycerin)') ORDER BY step.\n\
             3. Optionally gaps('research(tech=nitroglycerin)') first.\n\
             Rules: stubs not paths; read-only SQL; use_save before plan().",
        )]
    }
}

#[tool_handler]
#[prompt_handler]
impl ServerHandler for Vic3McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .enable_completions()
                .build(),
        )
        .with_instructions(
            "Vic3 Analyzer MCP: discover saves → use_save → campaign_brief / query / preview_delta. \
             Stubs only (no paths). Resources: vic3://schema|saves|session|docs/*."
                .to_string(),
        )
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, rmcp::ErrorData> {
        Ok(ListResourcesResult {
            resources: vec![
                resource("vic3://schema", "schema", "Fact tables, columns, and TVFs"),
                resource("vic3://saves", "saves", "Current save catalog snapshot"),
                resource("vic3://session", "session", "Active save and defs status"),
                resource("vic3://docs/flow", "docs-flow", "Short agent flow markdown"),
                resource("vic3://docs/sql", "docs-sql", "SQL interface contract"),
                resource("vic3://docs/mcp", "docs-mcp", "MCP design / tool contract"),
            ],
            ..Default::default()
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, rmcp::ErrorData> {
        let uri = request.uri.as_str();
        let text = match uri {
            "vic3://schema" => schema_catalog_json().to_string(),
            "vic3://saves" => self.saves_resource_json().await?,
            "vic3://session" => self.session_resource_json().await?,
            "vic3://docs/flow" => FLOW_MARKDOWN.to_string(),
            "vic3://docs/sql" => DOC_SQL.to_string(),
            "vic3://docs/mcp" => DOC_MCP.to_string(),
            _ => {
                return Err(rmcp::ErrorData::resource_not_found(
                    "resource_not_found",
                    Some(json!({ "uri": uri })),
                ));
            }
        };
        let mime = if uri.starts_with("vic3://docs/") {
            "text/markdown"
        } else {
            "application/json"
        };
        Ok(
            ReadResourceResult::new(vec![ResourceContents::text(text, uri).with_mime_type(mime)])
                .into(),
        )
    }

    async fn complete(
        &self,
        request: CompleteRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CompleteResult, rmcp::ErrorData> {
        // MCP completions cover prompts/resources (not tool args). Offer catalog
        // stubs when a prompt argument is named `name` / `stub` / `save`.
        let arg_name = request.argument.name.as_str();
        let partial = request.argument.value.to_lowercase();
        let values = match &request.r#ref {
            Reference::Prompt(_) if matches!(arg_name, "name" | "stub" | "save" | "save_name") => {
                self.catalog_stub_completions(&partial).await
            }
            Reference::Resource(r) if r.uri.contains("saves") && arg_name == "name" => {
                self.catalog_stub_completions(&partial).await
            }
            _ => {
                // Optional: table names from schema registry for SQL-ish prompts.
                if matches!(arg_name, "table" | "tables") {
                    vic3_sql::FACT_TABLES
                        .iter()
                        .map(|t| t.name().to_string())
                        .filter(|n| n.to_lowercase().contains(&partial))
                        .collect()
                } else {
                    Vec::new()
                }
            }
        };

        let completion = CompletionInfo::with_pagination(values, None, false)
            .map_err(|e| rmcp::ErrorData::internal_error(e, None))?;
        Ok(CompleteResult::new(completion))
    }
}

impl Vic3McpServer {
    async fn saves_resource_json(&self) -> Result<String, rmcp::ErrorData> {
        let entries = self
            .runtime
            .catalog_entries()
            .await
            .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))?;
        // Strip absolute paths from the agent-facing resource.
        let rows: Vec<serde_json::Value> = entries
            .iter()
            .map(|e| {
                json!({
                    "name": e.name,
                    "kind": e.kind.as_str(),
                    "mtime": system_time_iso(e.mtime),
                    "in_game_date": e.in_game_date,
                    "country": e.country,
                    "location": e.location.as_str(),
                })
            })
            .collect();
        Ok(json!({ "saves": rows, "count": rows.len() }).to_string())
    }

    async fn session_resource_json(&self) -> Result<String, rmcp::ErrorData> {
        let active = self.runtime.active_session().await;
        let defs = self.runtime.defs_status();
        let body = json!({
            "active": active.as_ref().map(|a| json!({
                "name": a.name,
                "kind": a.kind,
                "in_game_date": a.in_game_date,
                "country": a.country,
                "loaded": a.loaded,
                "location": a.location,
            })),
            "defs": {
                // App-data defs path is intentional; token map paths stay out of session JSON.
                "ready": defs.ready,
                "path": defs.path.as_ref().map(|p| redact_home(p)),
                "detail": defs.detail,
            },
            "config_path": redact_home(self.runtime.config_path()),
        });
        Ok(body.to_string())
    }

    async fn catalog_stub_completions(&self, partial: &str) -> Vec<String> {
        let Ok(entries) = self.runtime.catalog_entries().await else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .into_iter()
            .map(|e| e.name)
            .filter(|n| n.to_lowercase().contains(partial))
            .collect();
        names.sort();
        names.dedup();
        names.truncate(CompletionInfo::MAX_VALUES);
        names
    }
}

fn resource(uri: &str, name: &str, description: &str) -> Resource {
    Resource::new(uri, name).with_description(description)
}

async fn resolve_preview_delta(
    args: &PreviewDeltaArgs,
    runtime: &McpRuntime,
) -> Result<serde_json::Value, String> {
    let has_sugar = args.building.is_some()
        || args.extra_levels.is_some()
        || args.building_id.is_some()
        || args.state_id.is_some();
    let has_delta = args.delta.is_some();
    if has_sugar && has_delta {
        return Err(
            "preview_delta: provide either sugar (building/extra_levels/…) or delta, not both"
                .into(),
        );
    }
    if !has_sugar && !has_delta {
        return Err(
            "preview_delta: provide sugar (building + extra_levels) or a delta object".into(),
        );
    }

    let delta = if let Some(delta) = &args.delta {
        delta.clone()
    } else {
        let binding = runtime
            .active_binding()
            .await
            .ok_or_else(|| "no active save; call use_save first".to_string())?;
        world_delta_from_sugar(
            &binding,
            args.building.as_deref(),
            args.extra_levels,
            args.building_id,
            args.state_id,
        )?
    };

    runtime.preview_delta(&delta).await
}

fn build_use_save_request(args: &UseSaveArgs) -> Result<UseSaveRequest, String> {
    let location = match &args.location {
        Some(s) => Some(SaveLocation::from_str(s)?),
        None => None,
    };
    let mtime = match &args.mtime {
        Some(s) => Some(parse_mtime(s)?),
        None => None,
    };
    Ok(UseSaveRequest {
        name: args.name.clone(),
        selector: args.selector.clone(),
        location,
        mtime,
    })
}

fn parse_mtime(s: &str) -> Result<SystemTime, String> {
    let dt = DateTime::parse_from_rfc3339(s)
        .or_else(|_| DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ"))
        .map_err(|e| format!("mtime must be ISO-8601: {e}"))?;
    let utc: DateTime<Utc> = dt.with_timezone(&Utc);
    Ok(SystemTime::UNIX_EPOCH + Duration::from_secs(utc.timestamp().max(0) as u64))
}

fn normalize_explain_sql(sql: &str) -> String {
    let trimmed = sql.trim();
    if trimmed.len() >= 7 && trimmed[..7].eq_ignore_ascii_case("explain") {
        trimmed.to_string()
    } else {
        format!("EXPLAIN {trimmed}")
    }
}

fn system_time_iso(t: SystemTime) -> String {
    let dt: DateTime<Utc> = t.into();
    dt.to_rfc3339()
}

/// Stringify a path for `vic3://session` (full display path in v1).
fn redact_home(path: &std::path::Path) -> String {
    path.display().to_string()
}
