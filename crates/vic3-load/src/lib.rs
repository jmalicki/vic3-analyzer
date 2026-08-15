//! Victoria 3 save loading (jomini + pdx-tools `vic3save`).
//! Product IR arrives in phase 3b; this crate is a workspace stub.

/// Envelope types from pdx-tools until our IR lands.
pub use vic3save;

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
