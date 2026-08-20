//! Early argv mode selection (before Tauri `run`).
//!
//! Keeping this module free of Tauri imports documents the invariant: mode
//! selection must not create a window. Parsed in `main` so [`Mode::Mcp`] never
//! reaches [`crate::run`] (no WebView). Unknown tokens currently fall back to
//! [`Mode::Gui`] (reserved for future subcommands).

/// Process mode selected from argv.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Open the desktop WebView (default).
    Gui,
    /// Stdio MCP server (`vic3-mcp` / rmcp) — must not open a window.
    Mcp,
}

impl Mode {
    /// Parse `argv` (including program name at index 0).
    ///
    /// - no args / `gui` → [`Mode::Gui`]
    /// - `mcp` → [`Mode::Mcp`]
    /// - anything else → [`Mode::Gui`] with the unknown token ignored for now
    ///
    /// # Arguments
    ///
    /// * `args` — iterator yielding the program name first, then optional mode.
    ///
    /// # Examples
    ///
    /// ```
    /// use vic3_analyzer_lib::Mode;
    /// assert_eq!(Mode::from_args(["vic3-analyzer"]), Mode::Gui);
    /// assert_eq!(Mode::from_args(["vic3-analyzer", "mcp"]), Mode::Mcp);
    /// ```
    ///
    /// First matching token wins; later args are left for future subcommands.
    pub fn from_args<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut iter = args.into_iter();
        let _program = iter.next();
        match iter.next().as_ref().map(|s| s.as_ref()) {
            Some("mcp") => Mode::Mcp,
            Some("gui") | None => Mode::Gui,
            Some(_) => Mode::Gui,
        }
    }

    /// `true` when this mode must never call Tauri `run` / create a window.
    pub const fn is_headless(self) -> bool {
        matches!(self, Mode::Mcp)
    }
}

#[cfg(test)]
mod tests {
    use super::Mode;

    #[test]
    fn default_is_gui() {
        assert_eq!(Mode::from_args(["vic3-analyzer"]), Mode::Gui);
        assert!(!Mode::Gui.is_headless());
    }

    #[test]
    fn explicit_gui() {
        assert_eq!(Mode::from_args(["vic3-analyzer", "gui"]), Mode::Gui);
    }

    #[test]
    fn mcp_mode_is_headless() {
        assert_eq!(Mode::from_args(["vic3-analyzer", "mcp"]), Mode::Mcp);
        assert!(Mode::Mcp.is_headless());
    }

    #[test]
    fn unknown_falls_back_to_gui() {
        assert_eq!(Mode::from_args(["vic3-analyzer", "wat"]), Mode::Gui);
    }
}
