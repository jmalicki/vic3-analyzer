//! Price equilibrium. Limitations will be documented on `solve` in phase 4.

/// Solver caveats copied into CLI JSON and the UI (phase 4 fills these in).
pub const LIMITATIONS: &[&str] = &[];

/// Crate version from Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_semver() {
        assert!(!super::version().is_empty());
    }
}
