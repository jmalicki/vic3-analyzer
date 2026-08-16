use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::DEFAULT_PRICE_RANGE;

/// Parsed Victoria 3 definitions used by the price solver and wasm UI.
///
/// Built with [`crate::load_from_path`] from a game install or fixture tree, or
/// with [`crate::decode_blob`] from a compact in-memory snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameDefs {
    /// `NEconomy.PRICE_RANGE` (typically `0.75`).
    pub price_range: f64,
    pub goods: BTreeMap<String, Good>,
    /// Localized display labels keyed by script id. Empty when localization
    /// was not included in the selected game files.
    pub labels: BTreeMap<String, String>,
    /// Good id → PNG icon decoded from the install's DDS textures. Empty when
    /// the icon folder was not part of the selected game files.
    pub icons: BTreeMap<String, Vec<u8>>,
    pub production_methods: BTreeMap<String, ProductionMethod>,
    pub pop_needs: BTreeMap<String, PopNeed>,
    /// Wealth level (1–99) → buy package.
    pub buy_packages: BTreeMap<u8, BuyPackage>,
    /// Culture id → obsessed good ids. Empty when the tree has no obsessions.
    pub obsessions: BTreeMap<String, Vec<String>>,
}

impl Default for GameDefs {
    fn default() -> Self {
        Self {
            price_range: DEFAULT_PRICE_RANGE,
            goods: BTreeMap::new(),
            labels: BTreeMap::new(),
            icons: BTreeMap::new(),
            production_methods: BTreeMap::new(),
            pop_needs: BTreeMap::new(),
            buy_packages: BTreeMap::new(),
            obsessions: BTreeMap::new(),
        }
    }
}

impl GameDefs {
    /// Base price for `good_id`, if that good was parsed.
    pub fn base_price(&self, good_id: &str) -> Option<f64> {
        self.goods.get(good_id).map(|g| g.base_price)
    }
}

/// A tradeable (or local) good and its scripted base price (`cost` in `common/goods`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Good {
    pub id: String,
    pub base_price: f64,
    /// Game-relative DDS path. The blob does not embed image bytes.
    pub texture: Option<String>,
}

/// A production method with goods inputs/outputs scraped from building modifiers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductionMethod {
    pub id: String,
    pub inputs: BTreeMap<String, f64>,
    pub outputs: BTreeMap<String, f64>,
}

/// A pop need category (`common/pop_needs`) and its substitution table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PopNeed {
    pub id: String,
    pub default_good: Option<String>,
    pub entries: Vec<NeedEntry>,
}

/// One substitutable good inside a [`PopNeed`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NeedEntry {
    pub good: String,
    pub weight: f64,
    pub min_supply_share: f64,
    pub max_supply_share: f64,
}

/// Wealth-level consumption package (`common/buy_packages`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuyPackage {
    pub wealth: u8,
    pub political_strength: f64,
    /// Need id → package value (Vic3: base-price units per 10k working pops).
    pub needs: BTreeMap<String, f64>,
}
