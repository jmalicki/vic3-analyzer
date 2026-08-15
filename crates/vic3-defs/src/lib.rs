//! Game definitions for Victoria 3: goods, `PRICE_RANGE`, production methods,
//! pop needs, buy packages, and cultural obsessions.
//!
//! # Data roots
//!
//! [`load_from_path`] accepts a **game install** (`<Victoria 3>/game/common/...`)
//! or a **fixture tree** that already has `common/` at its root. Expected
//! relative paths (documented on the loader):
//!
//! - `common/goods`
//! - `common/defines` (`NEconomy.PRICE_RANGE`)
//! - `common/production_methods`
//! - `common/pop_needs`
//! - `common/buy_packages`
//! - `common/cultures` (obsessions; may be empty)
//!
//! Clausewitz text is parsed with **jomini** (`Deserialize` / `JominiDeserialize`).
//! This crate does not implement a Clausewitz lexer.
//!
//! # Wasm
//!
//! There is no filesystem in wasm. Encode a [`GameDefs`] with [`encode_blob`]
//! (postcard) and ship the bytes; the UI calls [`decode_blob`].

mod blob;
mod error;
mod load;
mod substitution;
mod types;

pub use blob::{decode_blob, encode_blob, BLOB_VERSION};
pub use error::DefsError;
pub use load::{load_from_files, load_from_path};
pub use substitution::{clamp_supply_share, substitution_shares, substitution_weight};
pub use types::{BuyPackage, GameDefs, Good, NeedEntry, PopNeed, ProductionMethod};

/// Vanilla `NEconomy.PRICE_RANGE` when a defines file does not override it.
pub const DEFAULT_PRICE_RANGE: f64 = 0.75;

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
