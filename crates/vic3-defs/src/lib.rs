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
//! - `common/buildings`
//! - `common/building_groups`
//! - `common/pop_needs`
//! - `common/buy_packages`
//! - `common/cultures` (obsessions; may be empty)
//! - `common/coat_of_arms`, `common/flag_definitions`, `common/named_colors`
//! - `gfx/coat_of_arms/{patterns,colored_emblems,textured_emblems}`
//! - `gfx/interface/icons/{goods_icons,building_icons,...}` (allowlisted leaf dirs)
//! - `localization/english/goods_l_*.yml` and `countries_l_*.yml`
//!
//! Clausewitz text is parsed with **jomini** (`Deserialize` / `JominiDeserialize`).
//! This crate does not implement a Clausewitz lexer.
//!
//! # Wasm
//!
//! There is no filesystem in wasm. Encode a [`GameDefs`] with [`encode_blob`]
//! (postcard) and ship the bytes; the UI calls [`decode_blob`].

mod blob;
mod coa;
mod error;
mod goods;
mod icons;
mod load;
mod needs;
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
pub use path_rules::{classify_defs_path, DefsPathClass, COMMON_DIRS};
pub use substitution::{clamp_supply_share, substitution_shares, substitution_weight};
pub use types::{
    BuildingGroup, BuildingType, BuyPackage, FlagDefinition, GameDefs, Good, NeedEntry, PopNeed,
    ProductionMethod,
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
