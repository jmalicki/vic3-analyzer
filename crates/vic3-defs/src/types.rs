use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{GoodIdx, NeedIdx, NeedsVec, DEFAULT_PRICE_RANGE};

/// Parsed Victoria 3 definitions used by the price solver and wasm UI.
///
/// Built with [`crate::load_from_path`] from a game install or fixture tree,
/// [`crate::load_from_files`] / [`crate::DefsBuilder`] from an allowlisted
/// in-memory selection, or [`crate::decode_blob`] from a postcard snapshot
/// (wasm). Dense [`crate::GoodIdx`] / [`crate::NeedIdx`] vectors align with
/// [`Self::goods_order`] / [`Self::needs_order`].
///
/// Icons and flags are PNG bytes ready for the browser; DDS stays on disk
/// during load. Localization may be empty when English goods/country files
/// were not selected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameDefs {
    /// `NEconomy.PRICE_RANGE` (typically `0.75`).
    pub price_range: f64,
    /// Good ids in deterministic `common/goods` source-file order.
    pub goods_order: Vec<String>,
    /// Need ids in deterministic first-seen load order.
    pub needs_order: Vec<String>,
    pub goods: BTreeMap<String, Good>,
    /// Localized display labels keyed by script id. Empty when localization
    /// was not included in the selected game files. `$key$` substitutions are
    /// expanded and `@icon!` markers dropped: the matching PNG lives in
    /// [`Self::extra_icons`] (`pop:academics` from `pops_icons/academics.dds`).
    pub labels: BTreeMap<String, String>,
    /// Good id → PNG icon decoded from the install's DDS textures. Empty when
    /// the icon folder was not part of the selected game files.
    pub icons: BTreeMap<String, Vec<u8>>,
    /// Namespaced PNG icons (`building:id`, `pm:id`, `pop:id`, `alert:id`,
    /// `military:id`, `generic:id`) from other `gfx/interface/icons` leaf folders.
    /// Script ids are aliased onto texture stems (`pm:pm_bakery` from `bakeries.dds`).
    #[serde(default)]
    pub extra_icons: BTreeMap<String, Vec<u8>>,
    /// Coat-of-arms id → rendered PNG flag (small). Empty when CoA assets were
    /// not part of the selected game files.
    pub flags: BTreeMap<String, Vec<u8>>,
    /// Tag → prioritized flag definitions used to pick a current CoA.
    #[serde(default)]
    pub flag_defs: BTreeMap<String, Vec<FlagDefinition>>,
    pub production_methods: BTreeMap<String, ProductionMethod>,
    /// Building type id → state-panel metadata.
    pub buildings: BTreeMap<String, BuildingType>,
    /// Building group id → grouping and slot-capacity metadata.
    pub building_groups: BTreeMap<String, BuildingGroup>,
    /// Pop needs aligned with [`Self::needs_order`].
    pub pop_needs: Vec<PopNeed>,
    /// Wealth level (1–99) → buy package.
    pub buy_packages: BTreeMap<u8, BuyPackage>,
    /// Dense need values for wealth levels 1..=99 (interpolated from packages).
    /// Empty when there are no buy packages. Index `w - 1` holds wealth `w`.
    #[serde(default)]
    pub package_ladder: Vec<NeedsVec>,
    /// Culture id → obsessed good indices. Empty when the tree has no obsessions.
    pub obsessions: BTreeMap<String, Vec<GoodIdx>>,
    /// Profession id → qualification script summary (`common/pop_types`).
    /// Empty when those files were not in the selected game files.
    #[serde(default)]
    pub pop_types: BTreeMap<String, PopType>,
    /// Production-method group id → PM ids (`common/production_method_groups`).
    #[serde(default)]
    pub production_method_groups: BTreeMap<String, Vec<String>>,
    /// Technology id → cost / prerequisite metadata (`common/technology/technologies`).
    /// Empty when those files were not in the selected game files.
    #[serde(default)]
    pub technologies: BTreeMap<String, Technology>,
}

impl Default for GameDefs {
    fn default() -> Self {
        Self {
            price_range: DEFAULT_PRICE_RANGE,
            goods_order: Vec::new(),
            needs_order: Vec::new(),
            goods: BTreeMap::new(),
            labels: BTreeMap::new(),
            icons: BTreeMap::new(),
            extra_icons: BTreeMap::new(),
            flags: BTreeMap::new(),
            flag_defs: BTreeMap::new(),
            production_methods: BTreeMap::new(),
            buildings: BTreeMap::new(),
            building_groups: BTreeMap::new(),
            pop_needs: Vec::new(),
            buy_packages: BTreeMap::new(),
            package_ladder: Vec::new(),
            obsessions: BTreeMap::new(),
            pop_types: BTreeMap::new(),
            production_method_groups: BTreeMap::new(),
            technologies: BTreeMap::new(),
        }
    }
}

impl GameDefs {
    /// Index of `good_id` in [`Self::goods_order`], if known.
    pub fn index_of(&self, good_id: &str) -> Option<GoodIdx> {
        self.goods_order
            .iter()
            .position(|id| id == good_id)
            .map(GoodIdx::from_usize)
    }

    /// Index of `need_id` in [`Self::needs_order`], if known.
    pub fn need_index_of(&self, need_id: &str) -> Option<NeedIdx> {
        self.needs_order
            .iter()
            .position(|id| id == need_id)
            .map(NeedIdx::from_usize)
    }

    /// Base price for `good_id`, if that good was parsed.
    pub fn base_price(&self, good_id: &str) -> Option<f64> {
        self.goods.get(good_id).map(|g| g.base_price)
    }

    /// Base price for an indexed good.
    pub fn base_price_idx(&self, idx: GoodIdx) -> Option<f64> {
        self.good_by_index(idx)
            .and_then(|id| self.goods.get(id).map(|g| g.base_price))
    }

    /// Goods volume represented by one unit of saved state trade capacity.
    pub fn traded_quantity_idx(&self, idx: GoodIdx) -> Option<f64> {
        self.good_by_index(idx)
            .and_then(|id| self.goods.get(id).map(|g| g.traded_quantity))
    }

    /// Good id at the integer index used by saved building IO.
    pub fn good_by_index(&self, index: GoodIdx) -> Option<&str> {
        self.goods_order.get(index.as_usize()).map(String::as_str)
    }

    /// Need id at `index`.
    pub fn need_id_by_index(&self, index: NeedIdx) -> Option<&str> {
        self.needs_order.get(index.as_usize()).map(String::as_str)
    }

    /// Pop need definition at `index`.
    pub fn need_by_index(&self, index: NeedIdx) -> Option<&PopNeed> {
        self.pop_needs.get(index.as_usize())
    }

    /// Pop need definition by script id.
    pub fn pop_need(&self, need_id: &str) -> Option<&PopNeed> {
        self.need_index_of(need_id)
            .and_then(|idx| self.need_by_index(idx))
    }

    /// Rebuild [`Self::package_ladder`] from [`Self::buy_packages`].
    ///
    /// Call after manually assembling packages in tests.
    pub fn rebuild_package_ladder(&mut self) {
        self.package_ladder = build_package_ladder(&self.buy_packages, self.needs_order.len());
    }
}

/// Fill wealth levels 1..=99 by interpolating neighboring defined packages.
pub(crate) fn build_package_ladder(
    packages: &BTreeMap<u8, BuyPackage>,
    n_needs: usize,
) -> Vec<NeedsVec> {
    if packages.is_empty() || n_needs == 0 {
        return Vec::new();
    }
    let keys: Vec<u8> = packages.keys().copied().collect();
    let min_w = keys[0];
    let max_w = *keys.last().expect("non-empty keys");
    (1u8..=99)
        .map(|wealth| {
            let w = wealth.clamp(min_w, max_w);
            let mut lo = keys[0];
            let mut hi = keys[0];
            for &k in &keys {
                if k <= w {
                    lo = k;
                }
                if k >= w {
                    hi = k;
                    break;
                }
                hi = k;
            }
            let p_lo = &packages[&lo].needs;
            if lo == hi {
                return p_lo.aligned(n_needs);
            }
            let span = f64::from(hi) - f64::from(lo);
            if span <= 0.0 {
                return p_lo.aligned(n_needs);
            }
            let t = (f64::from(w) - f64::from(lo)) / span;
            NeedsVec::lerp(
                &p_lo.aligned(n_needs),
                &packages[&hi].needs.aligned(n_needs),
                t,
            )
        })
        .collect()
}

/// One selectable flag for a country tag (`common/flag_definitions`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlagDefinition {
    pub coa: String,
    pub priority: i32,
    /// Empty means always eligible. Otherwise the country needs any listed law.
    pub any_laws: Vec<String>,
    /// When true this definition is ignored during selection.
    pub unsupported_trigger: bool,
}

/// A tradeable (or local) good and its scripted base price (`cost` in `common/goods`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Good {
    pub id: String,
    pub base_price: f64,
    /// Goods volume moved per unit of post-1.9 state trade capacity.
    pub traded_quantity: f64,
    /// Game-relative DDS path. The blob does not embed image bytes.
    pub texture: Option<String>,
}

/// A production method with goods inputs/outputs scraped from building modifiers.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProductionMethod {
    pub id: String,
    pub inputs: Vec<(GoodIdx, f64)>,
    pub outputs: Vec<(GoodIdx, f64)>,
    /// `building_employment_{profession}_add` totals scraped from modifiers.
    #[serde(default)]
    pub employment: Vec<(String, f64)>,
    /// True when any modifier adds `state_education_access_*`.
    #[serde(default)]
    pub education_access: bool,
    /// True when any modifier name contains `qualifications`.
    #[serde(default)]
    pub qualifications_boost: bool,
}

/// A technology definition (`common/technology/technologies`).
///
/// Vic3 scripts store prerequisites as `unlocking_technologies`. Innovation
/// cost is often era-scoped in the full game; fixtures may set [`Self::cost`]
/// directly. [`None`] cost means planners fall back to a model constant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Technology {
    pub id: String,
    /// Innovation points to research this tech when present in script/fixture.
    #[serde(default)]
    pub cost: Option<f64>,
    /// Tech ids from `unlocking_technologies = { ... }` (must own all to start).
    #[serde(default)]
    pub prerequisites: Vec<String>,
}

/// A constructable building definition (`common/buildings`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildingType {
    pub id: String,
    pub group: Option<String>,
    pub city_type: Option<String>,
    /// Script ids from `production_method_groups = { ... }`.
    #[serde(default)]
    pub production_method_groups: Vec<String>,
    /// Construction points to complete one level (`required_construction` in
    /// script). [`None`] when the file omitted the field; planners may fall
    /// back to a model constant.
    #[serde(default)]
    pub required_construction: Option<f64>,
}

/// One profession from `common/pop_types`.
///
/// [`Self::qualifications`] is a static walk of the scripted-value block, not a
/// Vic3 interpreter: nested `if`/`limit`/`is_pop_type` and `literacy`/`wealth`
/// mentions are recorded, but full trigger logic is not evaluated.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PopType {
    pub id: String,
    #[serde(default)]
    pub can_always_hire: bool,
    #[serde(default)]
    pub qualifications: QualificationFactors,
}

/// Who can qualify into a profession, and which gates the script mentions.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct QualificationFactors {
    /// Script mentions `literacy` as a factor.
    #[serde(default)]
    pub literacy: bool,
    /// Script mentions `wealth` as a factor.
    #[serde(default)]
    pub wealth: bool,
    /// `subtract` next to wealth, or a `wealth < N` limit, when present.
    #[serde(default)]
    pub wealth_floor: Option<f64>,
    /// `is_pop_type` / `pop_type` multipliers (default 1 when mentioned without a number).
    #[serde(default)]
    pub source_multipliers: BTreeMap<String, f64>,
}

/// A Vic3 building group (`common/building_groups`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildingGroup {
    pub id: String,
    pub category: Option<String>,
    pub land_usage: Option<String>,
    pub always_possible: bool,
    pub default_building: Option<String>,
    pub parent_group: Option<String>,
}

/// A pop need category (`common/pop_needs`) and its substitution table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PopNeed {
    pub id: String,
    pub default_good: Option<GoodIdx>,
    pub entries: Vec<NeedEntry>,
}

/// One substitutable good inside a [`PopNeed`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NeedEntry {
    pub good: GoodIdx,
    pub weight: f64,
    pub min_supply_share: f64,
    pub max_supply_share: f64,
}

/// Wealth-level consumption package (`common/buy_packages`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuyPackage {
    pub wealth: u8,
    pub political_strength: f64,
    /// Dense need package values (Vic3: base-price units per 10k working pops).
    pub needs: NeedsVec,
}
