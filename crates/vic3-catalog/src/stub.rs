//! Filename stub normalization for agent-facing save handles.

/// Normalize a user-facing save name to a filename stub.
///
/// Accepts `autosave` or `autosave.v3`. Strips at most one trailing `.v3`
/// (case-insensitive). Rejects path separators and parent segments.
pub fn normalize_stub(raw: &str) -> Result<String, StubError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(StubError::Empty);
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err(StubError::PathLike(trimmed.to_string()));
    }
    let mut stub = trimmed.to_string();
    if let Some(stripped) = strip_one_v3(&stub) {
        stub = stripped;
    }
    if stub.is_empty() {
        return Err(StubError::Empty);
    }
    Ok(stub)
}

fn strip_one_v3(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    lower
        .strip_suffix(".v3")
        .map(|base| name[..base.len()].to_string())
}

/// Classify a stub into a coarse save kind without opening the file.
pub fn classify_kind(stub: &str) -> SaveKind {
    let lower = stub.to_ascii_lowercase();
    if lower == "autosave" || lower.starts_with("autosave_") || lower.starts_with("autosave-") {
        SaveKind::Autosave
    } else {
        SaveKind::Named
    }
}

/// Coarse save category for catalog / SQL `kind` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveKind {
    Autosave,
    Named,
    Ironman,
}

impl SaveKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Autosave => "autosave",
            Self::Named => "named",
            Self::Ironman => "ironman",
        }
    }
}

impl std::fmt::Display for SaveKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Invalid stub input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StubError {
    #[error("save stub is empty")]
    Empty,
    #[error("save stub must be a basename, not a path: {0}")]
    PathLike(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_one_v3() {
        assert_eq!(normalize_stub("autosave.v3").unwrap(), "autosave");
        assert_eq!(normalize_stub("autosave.V3").unwrap(), "autosave");
        assert_eq!(normalize_stub("My Campaign.v3").unwrap(), "My Campaign");
    }

    #[test]
    fn keeps_stub_without_extension() {
        assert_eq!(normalize_stub("autosave").unwrap(), "autosave");
    }

    #[test]
    fn rejects_paths() {
        assert!(matches!(
            normalize_stub("../autosave"),
            Err(StubError::PathLike(_))
        ));
        assert!(matches!(
            normalize_stub("folder/autosave.v3"),
            Err(StubError::PathLike(_))
        ));
    }

    #[test]
    fn classifies_autosave_variants() {
        assert_eq!(classify_kind("autosave"), SaveKind::Autosave);
        assert_eq!(classify_kind("autosave_exit"), SaveKind::Autosave);
        assert_eq!(classify_kind("Campaign"), SaveKind::Named);
    }
}
