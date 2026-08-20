//! Map [`vic3_sql::SqlError`] into MCP tool-level errors agents can read.
//!
//! Prefer `CallToolResult::error` (tool ran, failed) over protocol `McpError`
//! so the message reaches the model (`docs/mcp.md`).

use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::json;
use vic3_sql::SqlError;

pub(crate) fn tool_err(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

pub(crate) fn tool_ok_json(value: &serde_json::Value) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(value.to_string())])
}

pub(crate) fn tool_ok_text(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(text.into())])
}

pub(crate) fn sql_to_tool_result(err: SqlError) -> CallToolResult {
    match &err {
        SqlError::Ambiguous { stub, candidates } => {
            let payload = json!({
                "error": "ambiguous_save",
                "stub": stub,
                "message": err.to_string(),
                "candidates": candidates.iter().map(|c| json!({
                    "name": c.name,
                    "kind": c.kind.as_str(),
                    "mtime": system_time_iso(c.mtime),
                    "location": c.location.as_str(),
                })).collect::<Vec<_>>(),
            });
            CallToolResult::error(vec![ContentBlock::text(payload.to_string())])
        }
        other => tool_err(other.to_string()),
    }
}

fn system_time_iso(t: std::time::SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Utc> = t.into();
    dt.to_rfc3339()
}
