//! Agent-facing save location tags (`local` | `steam_cloud`).

/// Where a save file was found (catalog / SQL `location` column).
///
/// Used to disambiguate the same filename stub under Documents vs Steam Cloud.
/// Serialize as snake_case strings for MCP / SQL / UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveLocation {
    /// Paradox Interactive Documents (or configured non-cloud root).
    Local,
    /// Steam Cloud cache under `userdata/<id>/529340/remote/save games`.
    SteamCloud,
}

impl SaveLocation {
    /// Stable string for SQL / JSON (`local` | `steam_cloud`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::SteamCloud => "steam_cloud",
        }
    }
}

impl std::fmt::Display for SaveLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SaveLocation {
    type Err = String;

    /// Parse `local` or `steam_cloud`.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message for any other string.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local" => Ok(Self::Local),
            "steam_cloud" => Ok(Self::SteamCloud),
            other => Err(format!("unknown save location: {other}")),
        }
    }
}
