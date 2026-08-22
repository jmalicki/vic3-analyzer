//! Game definitions for Victoria 3: goods, `PRICE_RANGE`, production methods,
//! pop needs, buy packages, cultural obsessions, buildings (including
//! optional `required_construction`), technologies (cost / prerequisites),
//! and UI assets.
//!
//! # Role in the pipeline
//!
//! ```text
//! save (+ tokens) → vic3-load → IR
//! install / fixture / blob → vic3-defs → GameDefs
//! IR + GameDefs → vic3-prices → … → plan → vic3-api
//! ```
//!
//! Defs merge layers (code defaults → blob/install → file overlays) are
//! described in [`docs/planning.md`](../../../docs/planning.md#framework-seams)
//! and implemented by [`overlay`]. This crate does **not** parse saves; it
//! turns Clausewitz game data (or a postcard blob) into [`GameDefs`].
//! Clausewitz text is parsed with **jomini**; there is no custom lexer.
//!
//! Architecture overview: [`docs/architecture.md`](../../../docs/architecture.md).
//! Price formula and pop consumption:
//! [`docs/prices.md`](../../../docs/prices.md).
//! Substitution invariant **I4**:
//! [`docs/invariants.md`](../../../docs/invariants.md).
//!
//! # Install vs fixture vs blob
//!
//! | Source | Entry point | When |
//! | --- | --- | --- |
//! | Game install (`…/Victoria 3/game/…`) or fixture tree with `common/` at root | [`load_from_path`] | CLI / desktop / tests with a filesystem |
//! | In-memory allowlisted files | [`load_from_files`] / [`DefsBuilder`] | Browser file picker; batched CoA art |
//! | Postcard snapshot | [`encode_blob`] / [`decode_blob`] | wasm: no filesystem; UI ships bytes |
//!
//! `goods_order` follows deterministic `common/goods` source-file order so
//! saved building IO indices line up with vanilla. Dense [`GoodIdx`] /
//! [`NeedIdx`] vectors are preferred over string keys in hot paths.
//!
//! # Path allowlist
//!
//! [`classify_defs_path`] is the trust boundary for browser walks and
//! [`load_from_files`]: only listed `common/` dirs, English localization
//! prefixes, and allowlisted icon / CoA leaf folders are read. See
//! [`COMMON_DIRS`].
//!
//! # Wasm
//!
//! There is no filesystem in wasm. Encode a [`GameDefs`] with [`encode_blob`]
//! (postcard, versioned by [`BLOB_VERSION`]) and ship the bytes; the UI calls
//! [`decode_blob`]. Bump [`BLOB_VERSION`] when [`GameDefs`] is not backward
//! compatible.

mod blob;
mod coa;
mod error;
mod goods;
mod icons;
mod load;
mod loc;
mod needs;
mod overlay;
mod path_rules;
mod staging;
mod substitution;
mod types;

pub use blob::{decode_blob, encode_blob, BLOB_VERSION};
pub use coa::{select_coa, select_flag_coa};
pub use error::DefsError;
pub use goods::{GoodIdx, GoodsVec};
pub use load::{load_from_files, load_from_path, DefsBuilder};
pub use needs::{NeedIdx, NeedsVec};
pub use overlay::{apply_overlay, load_overlay_json, BuildingOverlay, DefsOverlay};
pub use path_rules::{classify_defs_path, DefsPathClass, COMMON_DIRS};
pub use substitution::{clamp_supply_share, substitution_shares, substitution_weight};
pub use types::{
    BuildingGroup, BuildingType, BuyPackage, FlagDefinition, GameDefs, Good, NeedEntry, PopNeed,
    PopType, ProductionMethod, QualificationFactors, Technology,
};

/// Vanilla `NEconomy.PRICE_RANGE` when a defines file does not override it.
pub const DEFAULT_PRICE_RANGE: f64 = 0.75;

/// Vanilla `GOODS_DEFAULT_TRADE_QUANTITY` used when a good omits
/// `traded_quantity`.
pub const DEFAULT_TRADED_QUANTITY: f64 = 10.0;

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
