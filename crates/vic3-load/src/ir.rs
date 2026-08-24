//! Product IR deserialized from a Vic3 save via jomini / `DeserializeVic3`.
//!
//! # Ownership and growth
//!
//! Field names follow the save. This crate owns the structs so later phases can
//! add managers without forking pdx-tools `Vic3Save`. Missing keys default so a
//! small fixture (and older patches) still load. Extra save keys are ignored.
//!
//! # Sparse ids and `none`
//!
//! Paradox `*_manager.database` maps are sparse `u32` → object-or-`none`.
//! [`Manager`] skips deleted slots via [`Manager::iter_present`]. Binary saves
//! often store culture / goods as integer indices; fixtures use script ids —
//! flex deserializers accept both.
//!
//! # [`Save`] vs [`WorldSave`] vs [`WorldSnapshot`]
//!
//! - [`Save`] — full file-shaped IR for summaries and tests.
//! - [`WorldSave`] — subset deserialize for prices / planning (skips markets /
//!   trade routes; keeps tech, queues, interest, armies).
//! - [`WorldSnapshot`] — trait both implement so `vic3_prices::World::from_save`
//!   never depends on unused managers.
//!
//! Projection helpers ([`researched_techs_for`], [`army_power_projection_for`],
//! [`declared_interest_for`], …) read through [`WorldSnapshot`] for planning.

use crate::maybe::maybe_map;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use vic3save::Vic3Date;

/// Paradox `foo_manager.database` (or `states.database`, `pops.database`, …):
/// each id is either an object or `none`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(bound = "T: Deserialize<'de>")]
pub struct Manager<T> {
    /// 1.13 writes this map as `lod` rather than `database`.
    #[serde(default, alias = "lod", deserialize_with = "maybe_map")]
    pub database: HashMap<u32, Option<T>>,
}

impl<T> Default for Manager<T> {
    fn default() -> Self {
        Self {
            database: HashMap::new(),
        }
    }
}

impl<T> Manager<T> {
    /// Live entries (skips `none` slots).
    pub fn iter_present(&self) -> impl Iterator<Item = (u32, &T)> {
        self.database
            .iter()
            .filter_map(|(&id, slot)| slot.as_ref().map(|value| (id, value)))
    }
}

/// Top-level save IR. Thin overlay we own so later phases can add fields
/// without forking pdx-tools `Vic3Save`.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct Save {
    #[serde(default)]
    pub meta_data: Meta,
    #[serde(default)]
    pub country_manager: Manager<Country>,
    #[serde(default)]
    pub states: Manager<State>,
    #[serde(default)]
    pub building_manager: Manager<Building>,
    #[serde(default)]
    pub pops: Manager<Pop>,
    /// Index → script id (`0={ type=north_german }`). Not file order in `common/cultures`.
    #[serde(default)]
    pub cultures: Manager<Culture>,
    #[serde(default)]
    pub laws: Manager<LawEntry>,
    #[serde(default, alias = "markets")]
    pub market_manager: Manager<Market>,
    #[serde(default, alias = "trade_routes")]
    pub trade_route_manager: Manager<TradeRoute>,
    #[serde(default, alias = "constructions")]
    pub building_constructions: Manager<ConstructionOrder>,
    #[serde(default, alias = "gov_constructions")]
    pub government_constructions: Manager<ConstructionOrder>,
    /// Per-country researched techs. Real 1.5+ saves use this top-level
    /// manager (`acquired_technologies.value`, `research_technology`).
    #[serde(default)]
    pub technology: Manager<TechnologyEntry>,
    /// Declared / automatic interest markers (`interest_marker_manager`).
    #[serde(default, alias = "interest_marker_mgr", alias = "interest_markers")]
    pub interest_marker_manager: Manager<InterestMarker>,
    /// Formations when the save uses this key (1.5+ also writes
    /// `military_formation_manager`).
    #[serde(default, alias = "military_formation_manager")]
    pub formation_manager: Manager<MilitaryFormation>,
    #[serde(default)]
    pub military_formations: Manager<MilitaryFormation>,
    #[serde(default)]
    pub armies: Manager<MilitaryFormation>,
    #[serde(default)]
    pub navy_manager: Manager<MilitaryFormation>,
    #[serde(default)]
    pub hq_manager: Manager<MilitaryHq>,
    #[serde(default)]
    pub mobilization: Manager<MobilizationEntry>,
    #[serde(default)]
    pub previous_played: Vec<Player>,
}

/// Save fields the market + planning projections need.
///
/// Markets and trade routes stay skipped (prices inject trade via state rows).
/// Technology and construction queues are kept so `vic3_prices::World` /
/// planning can project researched techs and queue heads without a second full
/// [`Save`] parse.
/// Interest markers and army/navy formations are kept so planning can project
/// `army_power_projection` / `navy_power_projection` / `interest_in`.
/// Skipping still does **not** avoid zlib inflate of a single-member
/// `gamestate` zip. Keep [`Save`] when market/trade-route managers are part of
/// the answer (`parse_save` counts).
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct WorldSave {
    #[serde(default)]
    pub meta_data: Meta,
    #[serde(default)]
    pub country_manager: Manager<Country>,
    #[serde(default)]
    pub states: Manager<State>,
    #[serde(default)]
    pub building_manager: Manager<Building>,
    #[serde(default)]
    pub pops: Manager<Pop>,
    #[serde(default)]
    pub cultures: Manager<Culture>,
    #[serde(default)]
    pub laws: Manager<LawEntry>,
    #[serde(default, alias = "constructions")]
    pub building_constructions: Manager<ConstructionOrder>,
    #[serde(default, alias = "gov_constructions")]
    pub government_constructions: Manager<ConstructionOrder>,
    #[serde(default)]
    pub technology: Manager<TechnologyEntry>,
    /// Interest markers so planning can project `interest_in` without a full [`Save`].
    #[serde(default, alias = "interest_marker_mgr", alias = "interest_markers")]
    pub interest_marker_manager: Manager<InterestMarker>,
    /// Army/navy formations used to fall back power projection.
    #[serde(default, alias = "military_formation_manager")]
    pub formation_manager: Manager<MilitaryFormation>,
    #[serde(default)]
    pub military_formations: Manager<MilitaryFormation>,
    #[serde(default)]
    pub armies: Manager<MilitaryFormation>,
    /// Navy formations when the save uses a dedicated `navy_manager`.
    #[serde(default)]
    pub navy_manager: Manager<MilitaryFormation>,
    #[serde(default)]
    pub previous_played: Vec<Player>,
}

/// Subset of [`Save`] / [`WorldSave`] that `vic3_prices::World::from_save` reads.
///
/// One projection API so prices code never takes a dependency on unused
/// managers. Implementations stay field-shaped; layout/skipping is the
/// deserialize backend.
pub trait WorldSnapshot {
    fn meta_data(&self) -> &Meta;
    fn country_manager(&self) -> &Manager<Country>;
    fn states(&self) -> &Manager<State>;
    fn building_manager(&self) -> &Manager<Building>;
    fn pops(&self) -> &Manager<Pop>;
    fn cultures(&self) -> &Manager<Culture>;
    fn previous_played(&self) -> &[Player];
    fn active_laws(&self, country_id: u32) -> Vec<&str>;
    fn technology(&self) -> &Manager<TechnologyEntry>;
    fn building_constructions(&self) -> &Manager<ConstructionOrder>;
    fn government_constructions(&self) -> &Manager<ConstructionOrder>;
    fn interest_marker_manager(&self) -> &Manager<InterestMarker>;
    fn formation_manager(&self) -> &Manager<MilitaryFormation>;
    fn military_formations(&self) -> &Manager<MilitaryFormation>;
    fn armies(&self) -> &Manager<MilitaryFormation>;
    fn navy_manager(&self) -> &Manager<MilitaryFormation>;
}

fn active_laws_in(laws: &Manager<LawEntry>, country_id: u32) -> Vec<&str> {
    laws.iter_present()
        .filter(|(_, entry)| {
            entry.country == Some(country_id) && entry.active == Some(true) && !entry.law.is_empty()
        })
        .map(|(_, entry)| entry.law.as_str())
        .collect()
}

macro_rules! impl_world_snapshot {
    ($ty:ty) => {
        impl WorldSnapshot for $ty {
            fn meta_data(&self) -> &Meta {
                &self.meta_data
            }
            fn country_manager(&self) -> &Manager<Country> {
                &self.country_manager
            }
            fn states(&self) -> &Manager<State> {
                &self.states
            }
            fn building_manager(&self) -> &Manager<Building> {
                &self.building_manager
            }
            fn pops(&self) -> &Manager<Pop> {
                &self.pops
            }
            fn cultures(&self) -> &Manager<Culture> {
                &self.cultures
            }
            fn previous_played(&self) -> &[Player] {
                &self.previous_played
            }
            fn active_laws(&self, country_id: u32) -> Vec<&str> {
                active_laws_in(&self.laws, country_id)
            }
            fn technology(&self) -> &Manager<TechnologyEntry> {
                &self.technology
            }
            fn building_constructions(&self) -> &Manager<ConstructionOrder> {
                &self.building_constructions
            }
            fn government_constructions(&self) -> &Manager<ConstructionOrder> {
                &self.government_constructions
            }
            fn interest_marker_manager(&self) -> &Manager<InterestMarker> {
                &self.interest_marker_manager
            }
            fn formation_manager(&self) -> &Manager<MilitaryFormation> {
                &self.formation_manager
            }
            fn military_formations(&self) -> &Manager<MilitaryFormation> {
                &self.military_formations
            }
            fn armies(&self) -> &Manager<MilitaryFormation> {
                &self.armies
            }
            fn navy_manager(&self) -> &Manager<MilitaryFormation> {
                &self.navy_manager
            }
        }
    };
}

impl_world_snapshot!(Save);
impl_world_snapshot!(WorldSave);

/// Save metadata (`meta_data`).
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct Meta {
    #[serde(default)]
    pub version: String,
    pub game_date: Option<Vic3Date>,
    #[serde(default)]
    pub name: Option<String>,
}

/// A country definition in `country_manager.database`.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct Country {
    #[serde(default, deserialize_with = "flex_str")]
    pub definition: String,
    #[serde(default, deserialize_with = "optional_flex_str")]
    pub government: Option<String>,
    #[serde(default)]
    pub capital: Option<u32>,
    #[serde(default)]
    pub states: Vec<u32>,
    #[serde(default)]
    pub infamy: Option<f64>,
    #[serde(default)]
    pub budget: Budget,
    #[serde(default)]
    pub market: Option<u32>,
    #[serde(default)]
    pub market_capital: Option<u32>,
    #[serde(default)]
    pub is_main_tag: Option<bool>,
    #[serde(default, deserialize_with = "optional_flex_str")]
    pub country_type: Option<String>,
    /// Direct overlord country id when this country is a subject.
    #[serde(default)]
    pub overlord: Option<u32>,
    /// Subject type id when present (`puppet`, `dominion`, …).
    #[serde(default, deserialize_with = "optional_flex_str")]
    pub subject_type: Option<String>,
    /// Researched technology ids when the save stores them on the country.
    ///
    /// Real saves usually keep techs in the top-level [`Save::technology`]
    /// manager; [`Save::hydrate_country_techs`] copies those onto this field.
    #[serde(default, deserialize_with = "deserialize_tech_ids")]
    pub techs: Vec<String>,
    #[serde(default, deserialize_with = "optional_flex_str")]
    pub currently_researching: Option<String>,
    /// Nested `technology={ acquired=… }` / list of ids, when present.
    #[serde(default, deserialize_with = "deserialize_tech_ids")]
    pub technology: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_tech_ids")]
    pub research: Vec<String>,
    /// Cached army power projection when the save stores it on the country.
    #[serde(
        default,
        alias = "army_power_projection",
        alias = "country_army_power_projection"
    )]
    pub cached_total_army_power_projection: Option<f64>,
    /// Cached navy power projection when the save stores it on the country.
    #[serde(
        default,
        alias = "navy_power_projection",
        alias = "country_navy_power_projection",
        alias = "cached_total_navy_power_projection"
    )]
    pub cached_total_navy_power_projection: Option<f64>,
    /// Declared strategic-region interest ids when listed on the country.
    #[serde(default, deserialize_with = "deserialize_tech_ids")]
    pub declared_interests: Vec<String>,
}

impl Country {
    /// Researched ids from every country-level tech field we know.
    pub fn researched_techs(&self) -> Vec<String> {
        let mut ids = Vec::new();
        for id in self
            .techs
            .iter()
            .chain(self.technology.iter())
            .chain(self.research.iter())
        {
            if !id.is_empty() && !ids.iter().any(|seen| seen == id) {
                ids.push(id.clone());
            }
        }
        ids
    }
}

/// Treasury / credit lines when present on a country.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct Budget {
    #[serde(default)]
    pub gold_reserves: Option<f64>,
    #[serde(default)]
    pub gold: Option<f64>,
    #[serde(default)]
    pub money: Option<f64>,
    /// Credit limit when present.
    #[serde(default)]
    pub credit: Option<f64>,
    /// Outstanding debt principal when present.
    #[serde(default)]
    pub principal: Option<f64>,
    #[serde(default)]
    pub weekly_income: Vec<f64>,
}

impl Budget {
    /// Best-effort cash on hand (`gold_reserves`, then `gold`, then `money`).
    pub fn treasury(&self) -> Option<f64> {
        self.gold_reserves.or(self.gold).or(self.money)
    }

    /// Remaining credit before the limit, when both principal and credit exist.
    pub fn credit_headroom(&self) -> Option<f64> {
        match (self.principal, self.credit) {
            (Some(principal), Some(credit)) if principal.is_finite() && credit.is_finite() => {
                Some(credit - principal)
            }
            _ => None,
        }
    }

    /// Known remaining credit before exhaustion (`principal < credit`).
    ///
    /// Missing principal or credit leaves solvency unknown, so this returns
    /// `false` rather than guessing from treasury sign.
    pub fn is_solvent(&self) -> bool {
        self.credit_headroom()
            .map(|headroom| headroom > 0.0)
            .unwrap_or(false)
    }
}

/// A state in `states.database`.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct State {
    #[serde(default, deserialize_with = "optional_flex_str")]
    pub region: Option<String>,
    #[serde(default)]
    pub country: Option<u32>,
    #[serde(default)]
    pub market: Option<u32>,
    #[serde(default)]
    pub arable_land: Option<f64>,
    #[serde(default)]
    pub infrastructure: Option<f64>,
    #[serde(default)]
    pub infrastructure_usage: Option<f64>,
    /// Signed post-1.9 world-market trade capacity by goods-table index.
    ///
    /// Positive values are imports into the state; negative values are exports
    /// from it. Multiply by the good's `traded_quantity` for goods volume.
    #[serde(default)]
    pub trade: BuildingGoods,
    /// Profession-index (or id) → qualification stock. Vic3 prefixes the map
    /// with the pop-type count (`{ 15 0=… 7=… }`); 0 is `academics`.
    #[serde(default)]
    pub qualifications: IndexQtyMap,
    /// Hireable subset of [`Self::qualifications`] when the save stores it.
    #[serde(default)]
    pub employable_qualifications: IndexQtyMap,
    /// Employed workforce by profession index, when present on the state.
    #[serde(default)]
    pub pop_workforce_by_type: IndexQtyMap,
    /// 1.13 keeps the profession tables here rather than on the state itself.
    #[serde(default)]
    pub pop_statistics: StatePopStatistics,
}

impl State {
    /// Hireable qualification stock by profession, from wherever the save keeps it.
    pub fn employable(&self) -> &IndexQtyMap {
        if self.employable_qualifications.values.is_empty() {
            &self.pop_statistics.population_employable_qualifications
        } else {
            &self.employable_qualifications
        }
    }

    /// Employed workforce by profession, from wherever the save keeps it.
    pub fn workforce_by_profession(&self) -> &IndexQtyMap {
        if self.pop_workforce_by_type.values.is_empty() {
            &self.pop_statistics.population_workforce_by_profession
        } else {
            &self.pop_workforce_by_type
        }
    }
}

/// `pop_statistics` block on a 1.13 state.
///
/// The state itself has no `qualifications=`; the profession tables live here
/// under `population_*` names, each a size-prefixed index map (`{ 15 0=… }`).
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct StatePopStatistics {
    /// Profession index → people (workforce plus dependents).
    #[serde(default)]
    pub population_by_profession: IndexQtyMap,
    /// Profession index → working people.
    #[serde(default)]
    pub population_workforce_by_profession: IndexQtyMap,
    /// Profession index → people qualified and available to hire.
    #[serde(default)]
    pub population_employable_qualifications: IndexQtyMap,
}

/// A building in `building_manager.database`.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct Building {
    #[serde(default, deserialize_with = "flex_str")]
    pub building: String,
    #[serde(default)]
    pub state: Option<u32>,
    #[serde(default, rename = "levels", alias = "level")]
    pub level: i32,
    /// Staffed levels, up to [`Self::level`], rather than a 0–1 fraction.
    #[serde(default)]
    pub staffing: f64,
    /// Active production method when a save records a single id.
    #[serde(default, deserialize_with = "optional_flex_str")]
    pub production_method: Option<String>,
    /// Active production methods, one per PM group. This is the shape a real
    /// save uses; a building runs every listed method at once.
    ///
    /// For Construction Sector rows these ids are **required** and must resolve
    /// in defs to a PM with `country_construction_add` (see planning
    /// construction throughput). String script ids vs a bidirectional index map
    /// remains an open design question upstream of this IR.
    #[serde(default, deserialize_with = "flex_str_vec")]
    pub production_methods: Vec<String>,
    /// Saved input volumes when present. Keys are good script ids (fixtures) or
    /// the vanilla goods-table index as a decimal string (real saves).
    #[serde(default)]
    pub input_goods: BuildingGoods,
    /// Saved output volumes when present.
    #[serde(default)]
    pub output_goods: BuildingGoods,
}

/// Sparse Clausewitz map of id-or-index → quantity.
///
/// Real 1.13 pops/states store `qualifications={ 15 0=1.64 7=6.73 }` (leading
/// integer is the pop-type table size; 0 = academics). Fixtures may omit the
/// prefix or use script ids.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IndexQtyMap {
    pub values: BTreeMap<String, f64>,
}

impl<'de> Deserialize<'de> for IndexQtyMap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = IndexQtyMap;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a Clausewitz map of id = quantity, optionally size-prefixed")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = BTreeMap::new();
                while let Some(key) = map.next_key::<FlexStr>()? {
                    if key.0 == "remainder" {
                        let _: de::IgnoredAny = map.next_value()?;
                        continue;
                    }
                    let qty = map.next_value::<GoodQty>()?;
                    values.insert(key.0, qty.value());
                }
                Ok(IndexQtyMap { values })
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = BTreeMap::new();
                let mut pending: Option<String> = None;
                while let Some(token) = seq.next_element::<SeqTok>()? {
                    match token {
                        SeqTok::Skip => {}
                        SeqTok::Ident(key) if key == "=" => {
                            let key = pending.take().ok_or_else(|| {
                                de::Error::custom("qualification assignment missing key")
                            })?;
                            if key == "remainder" {
                                let _: Option<de::IgnoredAny> = seq.next_element()?;
                                continue;
                            }
                            let qty = seq.next_element::<GoodQty>()?.ok_or_else(|| {
                                de::Error::custom("qualification assignment missing value")
                            })?;
                            values.insert(key, qty.value());
                        }
                        SeqTok::Ident(key) => pending = Some(key),
                        SeqTok::Float(qty) => {
                            if let Some(key) = pending.take() {
                                if key != "remainder" {
                                    values.insert(key, qty);
                                }
                            }
                        }
                    }
                }
                Ok(IndexQtyMap { values })
            }
        }
        deserializer.deserialize_any(V)
    }
}

/// Goods quantities on a building (`input_goods` / `output_goods`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BuildingGoods {
    pub goods: BTreeMap<String, f64>,
}

impl<'de> Deserialize<'de> for BuildingGoods {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            goods: std::collections::BTreeMap<FlexStr, GoodQty>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Self {
            goods: raw
                .goods
                .into_iter()
                .map(|(key, qty)| (key.0, qty.value()))
                .filter(|(_, qty)| *qty != 0.0)
                .collect(),
        })
    }
}

/// Float or `{ value = … }` (Vic3 1.9+).
#[derive(Debug, Clone, PartialEq)]
struct GoodQty(f64);

impl GoodQty {
    fn value(self) -> f64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for GoodQty {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = GoodQty;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a number or { value = … }")
            }
            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Ok(GoodQty(value))
            }
            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(GoodQty(value as f64))
            }
            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(GoodQty(value as f64))
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                value.parse().map(GoodQty).map_err(E::custom)
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut value = None;
                while let Some(key) = map.next_key::<FlexStr>()? {
                    if key.0 == "value" {
                        value = Some(map.next_value::<f64>()?);
                    } else {
                        let _: serde::de::IgnoredAny = map.next_value()?;
                    }
                }
                value
                    .map(GoodQty)
                    .ok_or_else(|| serde::de::Error::custom("missing building good value"))
            }
            // Jomini exposes a single-field Clausewitz object as a sequence
            // when it is nested below an integer map key.
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut value = None;
                while let Some(token) = seq.next_element::<SeqTok>()? {
                    match token {
                        SeqTok::Float(qty) => value = Some(qty),
                        SeqTok::Ident(key) if key == "=" => {
                            if let Some(SeqTok::Float(qty)) = seq.next_element::<SeqTok>()? {
                                value = Some(qty);
                            }
                        }
                        SeqTok::Ident(key) if key == "value" => {
                            match seq.next_element::<SeqTok>()? {
                                Some(SeqTok::Ident(op)) if op == "=" => {
                                    value = seq.next_element::<f64>()?;
                                }
                                Some(SeqTok::Float(qty)) => value = Some(qty),
                                _ => {}
                            }
                        }
                        SeqTok::Ident(_) => {
                            let _ = seq.next_element::<serde::de::IgnoredAny>()?;
                        }
                        SeqTok::Skip => {}
                    }
                }
                value
                    .map(GoodQty)
                    .ok_or_else(|| serde::de::Error::custom("missing building good value"))
            }
        }
        deserializer.deserialize_any(V)
    }
}

impl Building {
    /// Every active production method, whichever field the save used.
    pub fn active_production_methods(&self) -> Vec<String> {
        let mut out = self.production_methods.clone();
        if let Some(single) = self.production_method.clone() {
            if !out.contains(&single) {
                out.push(single);
            }
        }
        out.retain(|id| !id.is_empty());
        out
    }
}

/// A culture in `cultures.database`.
///
/// Pops store a numeric index (`culture=0`); this table maps that index to a
/// script id. Localization labels are in the defs blob, keyed by that id.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct Culture {
    #[serde(default, rename = "type", deserialize_with = "flex_str")]
    pub id: String,
}

/// A pop in `pops.database`.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct Pop {
    #[serde(
        default,
        rename = "type",
        alias = "profession",
        deserialize_with = "optional_flex_str"
    )]
    pub profession: Option<String>,
    #[serde(default)]
    pub size: Option<f64>,
    #[serde(default, alias = "size_wa")]
    pub workforce: Option<f64>,
    #[serde(default, alias = "size_dn")]
    pub dependents: Option<f64>,
    #[serde(default)]
    pub wealth: Option<i32>,
    #[serde(default, deserialize_with = "optional_flex_str")]
    pub culture: Option<String>,
    #[serde(default, rename = "location", alias = "state")]
    pub state: Option<u32>,
    /// Frozen wage bill when present; omitted pops keep wealth at the saved integer.
    #[serde(default)]
    pub wages: Option<f64>,
    /// Building id this pop works at. Missing or `none` means unemployed.
    #[serde(default, deserialize_with = "optional_id")]
    pub workplace: Option<u32>,
    /// Literate workers (not a fraction). Alias `num_literate`.
    #[serde(default, alias = "num_literate")]
    pub literate: Option<f64>,
    /// Profession-index → people who could work that profession.
    #[serde(default)]
    pub qualifications: IndexQtyMap,
}

/// Integer or string map key / scalar. Binary saves write culture and goods
/// indices as integers; plaintext fixtures use script ids or decimal strings.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FlexStr(String);

impl<'de> Deserialize<'de> for FlexStr {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = FlexStr;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a string or integer")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(FlexStr(value.to_string()))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(FlexStr(value))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(FlexStr(value.to_string()))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(FlexStr(value.to_string()))
            }
        }
        deserializer.deserialize_any(V)
    }
}

enum SeqTok {
    Ident(String),
    Float(f64),
    Skip,
}

impl<'de> Deserialize<'de> for SeqTok {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = SeqTok;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a map key, '=', or quantity")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(SeqTok::Ident(value.to_string()))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(SeqTok::Ident(value))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(SeqTok::Ident(value.to_string()))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(SeqTok::Ident(value.to_string()))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Ok(SeqTok::Float(value))
            }
            fn visit_bool<E: de::Error>(self, _value: bool) -> Result<Self::Value, E> {
                Ok(SeqTok::Skip)
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(SeqTok::Skip)
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(SeqTok::Skip)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut value = None;
                while let Some(key) = map.next_key::<FlexStr>()? {
                    if key.0 == "value" {
                        value = Some(map.next_value::<f64>()?);
                    } else {
                        let _: de::IgnoredAny = map.next_value()?;
                    }
                }
                Ok(value.map(SeqTok::Float).unwrap_or(SeqTok::Skip))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut value = None;
                while let Some(token) = seq.next_element::<SeqTok>()? {
                    match token {
                        SeqTok::Float(qty) => value = Some(qty),
                        SeqTok::Ident(key) if key == "value" || key == "=" => {
                            if let Some(SeqTok::Float(qty)) = seq.next_element::<SeqTok>()? {
                                value = Some(qty);
                            }
                        }
                        SeqTok::Ident(_) | SeqTok::Skip => {}
                    }
                }
                Ok(value.map(SeqTok::Float).unwrap_or(SeqTok::Skip))
            }
        }
        deserializer.deserialize_any(V)
    }
}

fn flex_str<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    FlexStr::deserialize(deserializer).map(|value| value.0)
}

fn flex_str_vec<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    Vec::<FlexStr>::deserialize(deserializer)
        .map(|values| values.into_iter().map(|value| value.0).collect())
}

/// Clausewitz list of tech ids, or `{ value = { id id } }` (real 1.5+ saves).
fn deserialize_tech_ids<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    TechIds::deserialize(deserializer).map(|ids| ids.0)
}

#[derive(Debug, Clone, PartialEq, Default)]
struct TechIds(Vec<String>);

impl<'de> Deserialize<'de> for TechIds {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = TechIds;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a list of technology ids or { value = { … } }")
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(TechIds(Vec::new()))
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(TechIds(Vec::new()))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                if value.is_empty() || value.eq_ignore_ascii_case("none") {
                    Ok(TechIds(Vec::new()))
                } else {
                    Ok(TechIds(vec![value.to_string()]))
                }
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                self.visit_str(&value)
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(TechIds(vec![value.to_string()]))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(TechIds(vec![value.to_string()]))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut ids = Vec::new();
                while let Some(item) = seq.next_element::<TechIds>()? {
                    for id in item.0 {
                        if !id.is_empty() && !ids.iter().any(|seen| seen == &id) {
                            ids.push(id);
                        }
                    }
                }
                Ok(TechIds(ids))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut ids = Vec::new();
                while let Some(key) = map.next_key::<FlexStr>()? {
                    match key.0.as_str() {
                        "value"
                        | "acquired"
                        | "acquired_technologies"
                        | "researched"
                        | "techs"
                        | "discovered" => {
                            for id in map.next_value::<TechIds>()?.0 {
                                if !id.is_empty() && !ids.iter().any(|seen| seen == &id) {
                                    ids.push(id);
                                }
                            }
                        }
                        _ => {
                            let _: de::IgnoredAny = map.next_value()?;
                        }
                    }
                }
                Ok(TechIds(ids))
            }
        }
        deserializer.deserialize_any(V)
    }
}

/// Units listed as a vec, a `{ database = … }` manager, or a map of ids.
fn deserialize_units<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<MilitaryUnit>, D::Error> {
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Vec<MilitaryUnit>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a list or database of military units")
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(Vec::new())
        }
        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(Vec::new())
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut units = Vec::new();
            while let Some(unit) = seq.next_element::<FlexUnit>()? {
                if let Some(unit) = unit.0 {
                    units.push(unit);
                }
            }
            Ok(units)
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut units = Vec::new();
            while let Some(key) = map.next_key::<FlexStr>()? {
                if key.0 == "database" || key.0 == "lod" {
                    let UnitDatabase(database) = map.next_value()?;
                    for (id, slot) in database {
                        if let Some(mut unit) = slot {
                            unit.id = unit.id.or(Some(id));
                            units.push(unit);
                        }
                    }
                    continue;
                }
                if let Ok(id) = key.0.parse::<u32>() {
                    let crate::maybe::NoneOr(slot): crate::maybe::NoneOr<MilitaryUnit> =
                        map.next_value()?;
                    if let Some(mut unit) = slot {
                        unit.id = unit.id.or(Some(id));
                        units.push(unit);
                    }
                    continue;
                }
                let _: de::IgnoredAny = map.next_value()?;
            }
            Ok(units)
        }
    }
    deserializer.deserialize_any(V)
}

struct UnitDatabase(HashMap<u32, Option<MilitaryUnit>>);

impl<'de> Deserialize<'de> for UnitDatabase {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        maybe_map(deserializer).map(UnitDatabase)
    }
}

struct FlexUnit(Option<MilitaryUnit>);

impl<'de> Deserialize<'de> for FlexUnit {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = FlexUnit;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a military unit object, id, or none")
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(FlexUnit(None))
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(FlexUnit(None))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                if value.is_empty() || value.eq_ignore_ascii_case("none") {
                    return Ok(FlexUnit(None));
                }
                Ok(FlexUnit(Some(MilitaryUnit {
                    kind: Some(value.to_string()),
                    name: Some(value.to_string()),
                    ..MilitaryUnit::default()
                })))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                self.visit_str(&value)
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(FlexUnit(Some(MilitaryUnit {
                    id: u32::try_from(value).ok(),
                    ..MilitaryUnit::default()
                })))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(FlexUnit(Some(MilitaryUnit {
                    id: u32::try_from(value).ok(),
                    ..MilitaryUnit::default()
                })))
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                MilitaryUnit::deserialize(de::value::MapAccessDeserializer::new(map))
                    .map(|unit| FlexUnit(Some(unit)))
            }
            // Jomini often exposes nested `{ type=infantry … }` as a sequence.
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut unit = MilitaryUnit::default();
                let mut pending: Option<String> = None;
                while let Some(token) = seq.next_element::<SeqTok>()? {
                    match token {
                        SeqTok::Ident(key) if key == "=" => {
                            let key = pending.take().unwrap_or_default();
                            match seq.next_element::<SeqTok>()? {
                                Some(SeqTok::Ident(value)) => {
                                    apply_unit_ident(&mut unit, &key, value)
                                }
                                Some(SeqTok::Float(value)) => {
                                    apply_unit_float(&mut unit, &key, value)
                                }
                                Some(SeqTok::Skip) | None => {}
                            }
                        }
                        SeqTok::Ident(key) => pending = Some(key),
                        SeqTok::Float(value) => {
                            if let Some(key) = pending.take() {
                                apply_unit_float(&mut unit, &key, value);
                            } else {
                                unit.id = unit.id.or(u32::try_from(value as i64).ok());
                            }
                        }
                        SeqTok::Skip => {}
                    }
                }
                Ok(FlexUnit(Some(unit)))
            }
        }
        deserializer.deserialize_any(V)
    }
}

fn apply_unit_ident(unit: &mut MilitaryUnit, key: &str, value: String) {
    match key {
        "type" | "unit_type" => unit.kind = Some(value),
        "name" => unit.name = Some(value),
        "id" => unit.id = value.parse().ok(),
        _ => {}
    }
}

fn apply_unit_float(unit: &mut MilitaryUnit, key: &str, value: f64) {
    match key {
        "manpower" | "current_manpower" => unit.manpower = Some(value),
        "id" => unit.id = u32::try_from(value as i64).ok(),
        _ => {}
    }
}

fn optional_flex_str<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Option<String>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a string, integer, none, or omission")
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
            Ok(Some(value.to_string()))
        }
        fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
            Ok(Some(value))
        }
        fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
            Ok(Some(value.to_string()))
        }
        fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
            Ok(Some(value.to_string()))
        }
    }
    deserializer.deserialize_any(V)
}

fn optional_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u32>, D::Error> {
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Option<u32>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a building id, none, or omission")
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
            if value.eq_ignore_ascii_case("none") {
                return Ok(None);
            }
            value.parse().map(Some).map_err(E::custom)
        }
        fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
            u32::try_from(value).map(Some).map_err(E::custom)
        }
        fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
            u32::try_from(value).map(Some).map_err(E::custom)
        }
    }
    deserializer.deserialize_any(V)
}

impl Pop {
    /// Total household population used to scale demand.
    ///
    /// Current saves split pop size into `workforce` and `dependents`; older
    /// saves and fixtures use `size`, `size_wa`, and `size_dn`.
    pub fn demand_size(&self) -> Option<f64> {
        if self.workforce.is_some() || self.dependents.is_some() {
            let size = self.workforce.unwrap_or(0.0)
                + DEPENDENT_DEMAND_WEIGHT * self.dependents.unwrap_or(0.0);
            return (size > 0.0).then_some(size);
        }

        self.size.filter(|size| *size > 0.0)
    }
}

/// Dependents count as full household members for goods demand.
pub const DEPENDENT_DEMAND_WEIGHT: f64 = 1.0;

/// One entry in the top-level `technology.database` manager.
///
/// Garibaldi / melted 1.5+ saves store researched ids at
/// `acquired_technologies.value` and the queued tech at `research_technology`.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct TechnologyEntry {
    #[serde(default)]
    pub country: Option<u32>,
    #[serde(
        default,
        deserialize_with = "deserialize_tech_ids",
        alias = "acquired",
        alias = "researched",
        alias = "techs"
    )]
    pub acquired_technologies: Vec<String>,
    #[serde(
        default,
        deserialize_with = "optional_flex_str",
        alias = "currently_researching",
        alias = "researching"
    )]
    pub research_technology: Option<String>,
}

/// An army or navy formation (`formation_manager` / `military_formations`).
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct MilitaryFormation {
    #[serde(default, deserialize_with = "optional_flex_str")]
    pub name: Option<String>,
    #[serde(
        default,
        rename = "type",
        alias = "formation_type",
        deserialize_with = "optional_flex_str"
    )]
    pub kind: Option<String>,
    #[serde(default, alias = "owner")]
    pub country: Option<u32>,
    #[serde(default)]
    pub organization: Option<f64>,
    #[serde(default, alias = "manpower")]
    pub current_manpower: Option<f64>,
    /// Formation combat / power-projection contribution when present.
    #[serde(default, alias = "combat_power")]
    pub power_projection: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_units")]
    pub units: Vec<MilitaryUnit>,
}

/// One declared or automatic interest marker (`interest_marker_manager`).
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct InterestMarker {
    #[serde(default, alias = "owner")]
    pub country: Option<u32>,
    /// Strategic region script id (`region_western_europe`, `sr:…`, …).
    #[serde(
        default,
        alias = "region",
        alias = "strategic_region",
        deserialize_with = "optional_flex_str"
    )]
    pub strategic_region: Option<String>,
    /// State-region script id when the marker names a state directly.
    #[serde(default, deserialize_with = "optional_flex_str")]
    pub state: Option<String>,
    #[serde(default, rename = "type", deserialize_with = "optional_flex_str")]
    pub kind: Option<String>,
}

/// One unit listed on a formation. Fields are best-effort.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct MilitaryUnit {
    #[serde(default)]
    pub id: Option<u32>,
    #[serde(default, deserialize_with = "optional_flex_str")]
    pub name: Option<String>,
    #[serde(
        default,
        rename = "type",
        alias = "unit_type",
        deserialize_with = "optional_flex_str"
    )]
    pub kind: Option<String>,
    #[serde(default, alias = "current_manpower")]
    pub manpower: Option<f64>,
}

/// Headquarters row when `hq_manager` is present.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct MilitaryHq {
    #[serde(default, deserialize_with = "optional_flex_str")]
    pub name: Option<String>,
    #[serde(default)]
    pub country: Option<u32>,
    #[serde(default, deserialize_with = "optional_flex_str")]
    pub region: Option<String>,
}

/// Mobilization option / order when `mobilization` is present.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct MobilizationEntry {
    #[serde(default, deserialize_with = "optional_flex_str")]
    pub name: Option<String>,
    #[serde(default)]
    pub country: Option<u32>,
    #[serde(default, rename = "type", deserialize_with = "optional_flex_str")]
    pub kind: Option<String>,
}

/// A market in `market_manager.database`.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct Market {
    #[serde(default)]
    pub owner: Option<u32>,
}

/// One entry in the top-level `laws.database` manager.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct LawEntry {
    #[serde(default, deserialize_with = "flex_str")]
    pub law: String,
    #[serde(default)]
    pub country: Option<u32>,
    #[serde(default)]
    pub active: Option<bool>,
}

/// A trade route in `trade_route_manager.database`.
///
/// On the prices path these volumes are **frozen** (employment / wages / trade
/// centers stay fixed except explicit what-if deltas). [`WorldSave`] skips this
/// manager; post-1.9 trade capacity lives on [`State::trade`] instead.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct TradeRoute {
    #[serde(default)]
    pub goods: Option<String>,
    #[serde(default)]
    pub volume: Option<f64>,
    /// `true` = export (frozen sell), `false` = import (frozen buy). Missing → skip.
    #[serde(default, alias = "is_export")]
    pub export: Option<bool>,
}

/// A queued construction (private or government) when the save has one.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct ConstructionOrder {
    #[serde(default)]
    pub building: Option<String>,
    #[serde(default)]
    pub state: Option<u32>,
    #[serde(default)]
    pub remaining: Option<f64>,
}

/// Which save manager an order came from.
///
/// Vic3 keeps two independent queues. Planning's single `queued_building` head
/// prefers **private** over government (see [`queued_building_for`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ConstructionQueueKind {
    Private,
    Government,
}

impl ConstructionQueueKind {
    /// Stable SQL / JSON token (`private` / `government`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Government => "government",
        }
    }
}

impl std::fmt::Display for ConstructionQueueKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One present construction order with ownership resolved for SQL / UI / planning.
///
/// Distinct from the planning **head** (`queued_building`): that is only the
/// first private-then-government building type id. This row carries the full
/// queue entry (remaining construction, state, queue kind).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstructionQueueEntry {
    pub order_id: u32,
    pub queue: ConstructionQueueKind,
    /// Owner of [`Self::state_id`], when the state (or country states list) resolves.
    pub country_id: Option<u32>,
    pub state_id: Option<u32>,
    pub building: String,
    pub remaining: Option<f64>,
}

/// Last-played tag pointer (`previous_played`).
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct Player {
    #[serde(default)]
    pub idtype: Option<u32>,
    #[serde(default)]
    pub name: Option<String>,
}

impl Save {
    /// Countries that still exist (`none` slots skipped).
    pub fn countries(&self) -> impl Iterator<Item = (u32, &Country)> {
        self.country_manager.iter_present()
    }

    /// First country whose `definition` matches `tag`.
    pub fn country_by_tag(&self, tag: &str) -> Option<&Country> {
        self.countries()
            .find(|(_, country)| country.definition == tag)
            .map(|(_, country)| country)
    }

    /// Script id for a saved pop `culture` field.
    ///
    /// 1.13 writes an index into [`Self::cultures`]. Fixtures and older saves
    /// may already store `north_german`; those pass through unchanged.
    pub fn culture_id(&self, saved: Option<&str>) -> Option<String> {
        let saved = saved.filter(|value| !value.is_empty())?;
        if let Ok(index) = saved.parse::<u32>() {
            if let Some(culture) = self.cultures.database.get(&index).and_then(Option::as_ref) {
                if !culture.id.is_empty() {
                    return Some(culture.id.clone());
                }
            }
        }
        Some(saved.to_string())
    }

    /// Active law ids belonging to `country_id`.
    pub fn active_laws(&self, country_id: u32) -> Vec<&str> {
        WorldSnapshot::active_laws(self, country_id)
    }

    /// Copy researched ids from nested country fields and the top-level
    /// [`Self::technology`] manager onto each [`Country::techs`], and copy
    /// `research_technology` onto [`Country::currently_researching`] when unset.
    pub fn hydrate_country_techs(&mut self) {
        hydrate_country_techs(&mut self.country_manager, &self.technology);
    }

    /// Researched tech ids for `country_id` (country fields + technology manager).
    pub fn researched_techs_for(&self, country_id: u32) -> Vec<String> {
        researched_techs_for(self, country_id)
    }

    /// Research queue head for `country_id`, if any.
    pub fn queued_tech_for(&self, country_id: u32) -> Option<String> {
        queued_tech_for(self, country_id)
    }

    /// First construction queue head for buildings in states owned by `country_id`.
    pub fn queued_building_for(&self, country_id: u32) -> Option<String> {
        queued_building_for(self, country_id)
    }

    /// Army power projection for `country_id` (cached country value, else formations).
    pub fn army_power_projection_for(&self, country_id: u32) -> Option<f64> {
        army_power_projection_for(self, country_id)
    }

    /// Navy power projection for `country_id` (cached country value, else formations).
    pub fn navy_power_projection_for(&self, country_id: u32) -> Option<f64> {
        navy_power_projection_for(self, country_id)
    }

    /// Declared interest targets for `country_id`, split for DSL `state=` / `region=`.
    pub fn declared_interest_for(&self, country_id: u32) -> DeclaredInterest {
        declared_interest_for(self, country_id)
    }
}

impl WorldSave {
    /// Same hydration as [`Save::hydrate_country_techs`].
    pub fn hydrate_country_techs(&mut self) {
        hydrate_country_techs(&mut self.country_manager, &self.technology);
    }
}

/// Merge country-nested and top-level technology manager ids onto each country.
pub fn hydrate_country_techs(
    countries: &mut Manager<Country>,
    technology: &Manager<TechnologyEntry>,
) {
    for country in countries.database.values_mut().flatten() {
        country.techs = country.researched_techs();
    }
    let extra: Vec<(u32, Vec<String>, Option<String>)> = technology
        .iter_present()
        .filter_map(|(_, entry)| {
            entry.country.map(|country_id| {
                (
                    country_id,
                    entry.acquired_technologies.clone(),
                    entry.research_technology.clone(),
                )
            })
        })
        .collect();
    for (country_id, techs, researching) in extra {
        let Some(Some(country)) = countries.database.get_mut(&country_id) else {
            continue;
        };
        for tech in techs {
            if !tech.is_empty() && !country.techs.iter().any(|seen| seen == &tech) {
                country.techs.push(tech);
            }
        }
        if country.currently_researching.is_none() {
            if let Some(tech) = researching.filter(|id| !id.is_empty()) {
                country.currently_researching = Some(tech);
            }
        }
    }
}

/// Researched tech ids for a country from a [`WorldSnapshot`].
pub fn researched_techs_for(save: &impl WorldSnapshot, country_id: u32) -> Vec<String> {
    let mut ids = save
        .country_manager()
        .database
        .get(&country_id)
        .and_then(Option::as_ref)
        .map(Country::researched_techs)
        .unwrap_or_default();
    for (_, entry) in save.technology().iter_present() {
        if entry.country != Some(country_id) {
            continue;
        }
        for tech in &entry.acquired_technologies {
            if !tech.is_empty() && !ids.iter().any(|seen| seen == tech) {
                ids.push(tech.clone());
            }
        }
    }
    ids
}

/// Research queue head: country `currently_researching`, else technology manager.
pub fn queued_tech_for(save: &impl WorldSnapshot, country_id: u32) -> Option<String> {
    if let Some(tech) = save
        .country_manager()
        .database
        .get(&country_id)
        .and_then(Option::as_ref)
        .and_then(|country| country.currently_researching.clone())
        .filter(|id| !id.is_empty())
    {
        return Some(tech);
    }
    save.technology()
        .iter_present()
        .find_map(|(_, entry)| {
            (entry.country == Some(country_id))
                .then(|| entry.research_technology.clone())
                .flatten()
        })
        .filter(|id| !id.is_empty())
}

/// First private, then government, construction order in states owned by the country.
///
/// Private is preferred because the private queue is the economy's organic build
/// pipeline; government is the player's directed queue. Ownership is filtered via
/// state `country` (falling back to the country's `states` list).
pub fn queued_building_for(save: &impl WorldSnapshot, country_id: u32) -> Option<String> {
    constructions_for(save, country_id)
        .into_iter()
        .next()
        .map(|entry| entry.building)
}

/// All present construction orders for `country_id`, private then government, by `order_id`.
///
/// Skips empty building ids and orders whose state is not owned by the country.
pub fn constructions_for(
    save: &impl WorldSnapshot,
    country_id: u32,
) -> Vec<ConstructionQueueEntry> {
    let owned = owned_state_ids(save, country_id);
    let mut rows = Vec::new();
    collect_queue_orders(
        save.building_constructions(),
        ConstructionQueueKind::Private,
        Some((country_id, &owned)),
        &mut rows,
    );
    collect_queue_orders(
        save.government_constructions(),
        ConstructionQueueKind::Government,
        Some((country_id, &owned)),
        &mut rows,
    );
    rows
}

/// Every present construction order in the save (both queues), ordered private then government by id.
///
/// `country_id` is resolved from the order's state owner when possible.
pub fn all_constructions(save: &impl WorldSnapshot) -> Vec<ConstructionQueueEntry> {
    let state_owner: std::collections::HashMap<u32, u32> = save
        .states()
        .iter_present()
        .filter_map(|(id, state)| state.country.map(|country| (id, country)))
        .collect();
    // Country.states is a fallback when state rows omit `country`.
    let mut state_owner = state_owner;
    for (country_id, country) in save.country_manager().iter_present() {
        for &state_id in &country.states {
            state_owner.entry(state_id).or_insert(country_id);
        }
    }
    let mut rows = Vec::new();
    collect_queue_orders(
        save.building_constructions(),
        ConstructionQueueKind::Private,
        None,
        &mut rows,
    );
    collect_queue_orders(
        save.government_constructions(),
        ConstructionQueueKind::Government,
        None,
        &mut rows,
    );
    for row in &mut rows {
        if row.country_id.is_none() {
            if let Some(state_id) = row.state_id {
                row.country_id = state_owner.get(&state_id).copied();
            }
        }
    }
    rows
}

/// Declared interest ids split so DSL `interest_in(state=…)` / `region=` can differ.
///
/// State-region and strategic-region tokens are normalized separately via
/// [`normalize_interest_ids`] so planning atoms can match either form.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeclaredInterest {
    pub states: Vec<String>,
    pub regions: Vec<String>,
}

/// Army power projection for a country from save IR.
///
/// Prefers `cached_total_army_power_projection` / `army_power_projection` on the
/// country. Falls back to the sum of army-formation `power_projection` values
/// when the country cache is missing. Returns [`None`] when neither is present
/// (modern saves often omit both — do not treat that as zero strength).
pub fn army_power_projection_for(save: &impl WorldSnapshot, country_id: u32) -> Option<f64> {
    if let Some(cached) = save
        .country_manager()
        .database
        .get(&country_id)
        .and_then(Option::as_ref)
        .and_then(|country| country.cached_total_army_power_projection)
        .filter(|value| value.is_finite())
    {
        return Some(cached);
    }
    let mut total = 0.0;
    let mut found = false;
    for formation in save
        .formation_manager()
        .iter_present()
        .chain(save.military_formations().iter_present())
        .chain(save.armies().iter_present())
        .map(|(_, formation)| formation)
    {
        if formation.country != Some(country_id) {
            continue;
        }
        if is_navy_formation(formation.kind.as_deref()) {
            continue;
        }
        if let Some(power) = formation.power_projection.filter(|value| value.is_finite()) {
            total += power;
            found = true;
        }
    }
    if found {
        Some(total)
    } else {
        None
    }
}

fn is_navy_formation(kind: Option<&str>) -> bool {
    kind.is_some_and(|kind| {
        matches!(
            kind.to_ascii_lowercase().as_str(),
            "navy" | "fleet" | "flotilla" | "naval"
        )
    })
}

/// Navy power projection for a country from save IR.
///
/// Prefers `cached_total_navy_power_projection` / `navy_power_projection` on the
/// country. Falls back to the sum of navy-formation `power_projection` values
/// (including [`WorldSnapshot::navy_manager`]). Returns [`None`] when neither is
/// present — do not treat that as zero fleet strength.
pub fn navy_power_projection_for(save: &impl WorldSnapshot, country_id: u32) -> Option<f64> {
    if let Some(cached) = save
        .country_manager()
        .database
        .get(&country_id)
        .and_then(Option::as_ref)
        .and_then(|country| country.cached_total_navy_power_projection)
        .filter(|value| value.is_finite())
    {
        return Some(cached);
    }
    let mut total = 0.0;
    let mut found = false;
    for formation in save
        .formation_manager()
        .iter_present()
        .chain(save.military_formations().iter_present())
        .chain(save.armies().iter_present())
        .chain(save.navy_manager().iter_present())
        .map(|(_, formation)| formation)
    {
        if formation.country != Some(country_id) {
            continue;
        }
        if !is_navy_formation(formation.kind.as_deref()) {
            continue;
        }
        if let Some(power) = formation.power_projection.filter(|value| value.is_finite()) {
            total += power;
            found = true;
        }
    }
    if found {
        Some(total)
    } else {
        None
    }
}

/// Interest markers + country `declared_interests` for planning projection.
pub fn declared_interest_for(save: &impl WorldSnapshot, country_id: u32) -> DeclaredInterest {
    let mut states = Vec::new();
    let mut regions = Vec::new();
    let mut push_state = |raw: &str| {
        for id in normalize_interest_ids(raw) {
            if !states.iter().any(|seen| seen == &id) {
                states.push(id);
            }
        }
    };
    let mut push_region = |raw: &str| {
        for id in normalize_interest_ids(raw) {
            if !regions.iter().any(|seen| seen == &id) {
                regions.push(id);
            }
        }
    };

    if let Some(country) = save
        .country_manager()
        .database
        .get(&country_id)
        .and_then(Option::as_ref)
    {
        for id in &country.declared_interests {
            push_region(id);
        }
    }

    for (_, marker) in save.interest_marker_manager().iter_present() {
        if marker.country != Some(country_id) {
            continue;
        }
        if let Some(state) = marker.state.as_deref().filter(|id| !id.is_empty()) {
            push_state(state);
        }
        if let Some(region) = marker
            .strategic_region
            .as_deref()
            .filter(|id| !id.is_empty())
        {
            push_region(region);
        }
    }

    DeclaredInterest { states, regions }
}

/// Normalize Clausewitz interest ids for DSL matching.
///
/// Keeps the raw token and adds unprefixed / `STATE_`-stripped lowercase forms
/// so `interest_in(state=alsace)` matches `STATE_ALSACE` / `alsace`.
pub fn normalize_interest_ids(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return Vec::new();
    }
    let mut ids = vec![trimmed.to_string()];
    let without_sr = trimmed
        .strip_prefix("sr:")
        .or_else(|| trimmed.strip_prefix("SR:"))
        .unwrap_or(trimmed);
    if without_sr != trimmed {
        ids.push(without_sr.to_string());
    }
    let lower = without_sr.to_ascii_lowercase();
    if !ids.iter().any(|seen| seen == &lower) {
        ids.push(lower.clone());
    }
    if let Some(rest) = lower.strip_prefix("state_") {
        if !rest.is_empty() && !ids.iter().any(|seen| seen == rest) {
            ids.push(rest.to_string());
        }
    }
    if let Some(rest) = lower.strip_prefix("region_") {
        if !rest.is_empty() && !ids.iter().any(|seen| seen == rest) {
            ids.push(rest.to_string());
        }
    }
    ids
}

fn owned_state_ids(save: &impl WorldSnapshot, country_id: u32) -> std::collections::BTreeSet<u32> {
    // Union state.country rows with Country.states — some saves omit country on
    // a subset of state rows even when the country still lists those ids.
    let mut owned: std::collections::BTreeSet<u32> = save
        .states()
        .iter_present()
        .filter_map(|(id, state)| (state.country == Some(country_id)).then_some(id))
        .collect();
    if let Some(Some(country)) = save.country_manager().database.get(&country_id) {
        owned.extend(country.states.iter().copied());
    }
    owned
}

/// Collect present orders from one manager.
///
/// When `filter` is `Some((country_id, owned_states))`, only orders in those
/// states are kept and `country_id` is stamped on each row. When `None`, every
/// present order is kept (caller fills `country_id` from state ownership).
fn collect_queue_orders(
    orders: &Manager<ConstructionOrder>,
    queue: ConstructionQueueKind,
    filter: Option<(u32, &std::collections::BTreeSet<u32>)>,
    out: &mut Vec<ConstructionQueueEntry>,
) {
    let mut batch = Vec::new();
    for (order_id, order) in orders.iter_present() {
        let Some(building) = order.building.as_ref().filter(|id| !id.is_empty()).cloned() else {
            continue;
        };
        let state_id = order.state;
        let country_id = if let Some((country_id, owned)) = filter {
            let Some(state) = state_id else {
                continue;
            };
            if !owned.contains(&state) {
                continue;
            }
            Some(country_id)
        } else {
            None
        };
        batch.push(ConstructionQueueEntry {
            order_id,
            queue,
            country_id,
            state_id,
            building,
            remaining: order.remaining.filter(|value| value.is_finite()),
        });
    }
    batch.sort_by_key(|entry| entry.order_id);
    out.extend(batch);
}

#[cfg(test)]
mod tests {
    use super::{Budget, Pop};

    #[test]
    fn demand_size_uses_total_household_population() {
        let pop = Pop {
            workforce: Some(8_000.0),
            dependents: Some(4_000.0),
            ..Pop::default()
        };

        assert_eq!(pop.demand_size(), Some(12_000.0));
    }

    #[test]
    fn demand_size_falls_back_to_legacy_size() {
        let pop = Pop {
            size: Some(10_000.0),
            ..Pop::default()
        };

        assert_eq!(pop.demand_size(), Some(10_000.0));
    }

    #[test]
    fn manager_reads_lod_alias_used_by_1_13() {
        let save = crate::load_slice(
            br#"SAV01000000000000000000
country_manager={
	lod={
		1=none
		2={ definition="GER" }
	}
}
"#,
            crate::empty_tokens(),
        )
        .expect("lod manager");
        assert_eq!(save.country_manager.database.get(&1), Some(&None));
        assert_eq!(
            save.country_manager
                .database
                .get(&2)
                .and_then(Option::as_ref)
                .map(|country| country.definition.as_str()),
            Some("GER")
        );
    }

    #[test]
    fn qualifications_packed_array_skips_size_prefix() {
        let save = crate::load_slice(
            br#"SAV01000000000000000000
pops={
	database={
		1={
			qualifications={ 15 0=1.64121 2=2.76406 7=6.73474 }
		}
	}
}
"#,
            crate::empty_tokens(),
        )
        .expect("packed qualifications");
        let pop = save.pops.iter_present().next().unwrap().1;
        assert_eq!(pop.qualifications.values.get("0"), Some(&1.64121));
        assert_eq!(pop.qualifications.values.get("2"), Some(&2.76406));
        assert_eq!(pop.qualifications.values.get("7"), Some(&6.73474));
        assert!(!pop.qualifications.values.contains_key("15"));

        let nested = crate::load_slice(
            br#"SAV01000000000000000000
pops={
	database={
		1={
			qualifications={ 15 remainder={ 1 2 3 } 0={ value=1.64121 } 7=6.73474 }
		}
	}
}
"#,
            crate::empty_tokens(),
        )
        .expect("nested qualification quantities");
        let pop = nested.pops.iter_present().next().unwrap().1;
        assert_eq!(pop.qualifications.values.get("0"), Some(&1.64121));
        assert_eq!(pop.qualifications.values.get("7"), Some(&6.73474));
        assert!(!pop.qualifications.values.contains_key("remainder"));

        let named = crate::load_slice(
            br#"SAV01000000000000000000
pops={
	database={
		1={
			qualifications={ academics=10 farmers=5 }
		}
	}
}
"#,
            crate::empty_tokens(),
        )
        .expect("named qualifications");
        let pop = named.pops.iter_present().next().unwrap().1;
        assert_eq!(pop.qualifications.values.get("academics"), Some(&10.0));
        assert_eq!(pop.qualifications.values.get("farmers"), Some(&5.0));
    }

    #[test]
    fn state_profession_tables_prefer_flat_fields_over_pop_statistics() {
        let save = crate::load_slice(
            br#"SAV01000000000000000000
states={
	database={
		1={
			employable_qualifications={ 15 0=2 }
			pop_workforce_by_type={ 15 7=11 }
			pop_statistics={
				population_workforce_by_profession={ 15 7=6000 }
				population_employable_qualifications={ 15 0=50 }
			}
		}
	}
}
"#,
            crate::empty_tokens(),
        )
        .expect("state profession tables");
        let state = save.states.iter_present().next().unwrap().1;
        assert_eq!(state.employable().values.get("0"), Some(&2.0));
        assert_eq!(state.workforce_by_profession().values.get("7"), Some(&11.0));
    }

    #[test]
    fn culture_index_resolves_from_save_database() {
        let save = crate::load_slice(
            br#"SAV01000000000000000000
cultures={
	database={
		0={ type=north_german }
		1=none
		2={ type=ashkenazi }
	}
}
pops={
	database={
		1={ culture=0 }
		2={ culture="south_german" }
	}
}
"#,
            crate::empty_tokens(),
        )
        .expect("cultures database");
        let mut pops: Vec<_> = save.pops.iter_present().collect();
        pops.sort_by_key(|(id, _)| *id);
        assert_eq!(pops[0].1.culture.as_deref(), Some("0"));
        assert_eq!(
            save.culture_id(pops[0].1.culture.as_deref()).as_deref(),
            Some("north_german")
        );
        assert_eq!(
            save.culture_id(pops[1].1.culture.as_deref()).as_deref(),
            Some("south_german")
        );
        assert_eq!(save.culture_id(Some("2")).as_deref(), Some("ashkenazi"));
        assert_eq!(save.culture_id(Some("1")).as_deref(), Some("1"));
    }

    #[test]
    fn credit_headroom_requires_known_principal_and_limit() {
        let solvent = Budget {
            credit: Some(500.0),
            principal: Some(100.0),
            ..Budget::default()
        };
        assert_eq!(solvent.credit_headroom(), Some(400.0));
        assert!(solvent.is_solvent());

        let exhausted = Budget {
            credit: Some(500.0),
            principal: Some(500.0),
            gold_reserves: Some(10_000.0),
            ..Budget::default()
        };
        assert_eq!(exhausted.credit_headroom(), Some(0.0));
        assert!(!exhausted.is_solvent());

        let unknown = Budget {
            gold_reserves: Some(10_000.0),
            credit: Some(500.0),
            ..Budget::default()
        };
        assert_eq!(unknown.credit_headroom(), None);
        assert!(!unknown.is_solvent());
    }

    #[test]
    fn technology_manager_hydrates_country_techs() {
        let save = crate::load_slice(
            br#"SAV01000000000000000000
country_manager={
	database={
		16777216={ definition="GER" techs={ railways } }
	}
}
technology={
	database={
		1={
			country=16777216
			acquired_technologies={ value={ nitroglycerin urban_planning } }
			research_technology=atmospheric_engine
		}
	}
}
"#,
            crate::empty_tokens(),
        )
        .expect("technology manager");
        let ger = save.country_by_tag("GER").expect("GER");
        assert!(ger.techs.iter().any(|tech| tech == "railways"));
        assert!(ger.techs.iter().any(|tech| tech == "nitroglycerin"));
        assert!(ger.techs.iter().any(|tech| tech == "urban_planning"));
        assert_eq!(
            ger.currently_researching.as_deref(),
            Some("atmospheric_engine")
        );
        let entry = save.technology.iter_present().next().unwrap().1;
        assert_eq!(
            entry.research_technology.as_deref(),
            Some("atmospheric_engine")
        );
        assert_eq!(
            save.queued_tech_for(16777216).as_deref(),
            Some("atmospheric_engine")
        );
    }

    #[test]
    fn formation_manager_deserializes_army_and_navy() {
        let save = crate::load_slice(
            br#"SAV01000000000000000000
formation_manager={
	database={
		10={
			name="Army of the Elbe"
			type=army
			country=16777216
			organization=80
			current_manpower=12000
			units={ { type=infantry manpower=4000 } { type=artillery manpower=2000 } }
		}
		11=none
	}
}
navy_manager={
	database={
		20={ name="High Seas Fleet" type=navy country=16777216 organization=90 }
	}
}
hq_manager={
	database={
		1={ name="Berlin HQ" country=16777216 region=sr:north_german_plain }
	}
}
"#,
            crate::empty_tokens(),
        )
        .expect("formation manager");
        let army = save
            .formation_manager
            .iter_present()
            .find(|(_, formation)| formation.kind.as_deref() == Some("army"))
            .map(|(_, formation)| formation)
            .expect("army formation");
        assert_eq!(army.name.as_deref(), Some("Army of the Elbe"));
        assert_eq!(army.country, Some(16777216));
        assert_eq!(army.organization, Some(80.0));
        assert_eq!(army.current_manpower, Some(12000.0));
        assert_eq!(army.units.len(), 2);
        assert_eq!(army.units[0].kind.as_deref(), Some("infantry"));
        let navy = save.navy_manager.iter_present().next().unwrap().1;
        assert_eq!(navy.name.as_deref(), Some("High Seas Fleet"));
        assert_eq!(save.hq_manager.iter_present().count(), 1);
    }

    #[test]
    fn unknown_military_keys_do_not_fail_save_deserialize() {
        let save = crate::load_slice(
            br#"SAV01000000000000000000
military_gizmos={
	database={ 1={ nonsense=yes } }
}
country_manager={
	database={ 1={ definition="GER" } }
}
"#,
            crate::empty_tokens(),
        )
        .expect("unknown military keys");
        assert_eq!(save.country_by_tag("GER").unwrap().definition, "GER");
        assert_eq!(save.formation_manager.iter_present().count(), 0);
        assert_eq!(save.military_formations.iter_present().count(), 0);
    }

    #[test]
    fn army_power_and_interest_markers_deserialize() {
        let save = crate::load_slice(
            br#"SAV01000000000000000000
country_manager={
	database={
		16777216={
			definition="GER"
			cached_total_army_power_projection=180.5
			declared_interests={ region_north_africa }
		}
	}
}
interest_marker_manager={
	database={
		1={
			country=16777216
			strategic_region=sr:region_western_europe
			state=STATE_ALSACE
		}
		2=none
	}
}
formation_manager={
	database={
		10={
			type=army
			country=16777216
			power_projection=40
		}
	}
}
"#,
            crate::empty_tokens(),
        )
        .expect("army/interest ir");
        let ger = save.country_by_tag("GER").expect("GER");
        assert_eq!(ger.cached_total_army_power_projection, Some(180.5));
        assert_eq!(
            ger.declared_interests,
            vec!["region_north_africa".to_string()]
        );
        assert_eq!(save.army_power_projection_for(16777216), Some(180.5));
        let interest = save.declared_interest_for(16777216);
        assert!(interest.states.iter().any(|id| id == "alsace"));
        assert!(interest.states.iter().any(|id| id == "STATE_ALSACE"));
        assert!(interest
            .regions
            .iter()
            .any(|id| id == "region_western_europe"));
        assert!(interest
            .regions
            .iter()
            .any(|id| id == "region_north_africa"));
    }

    #[test]
    fn army_power_falls_back_to_formation_power_projection() {
        let save = crate::load_slice(
            br#"SAV01000000000000000000
country_manager={
	database={ 1={ definition="GER" } }
}
armies={
	database={
		1={ type=army country=1 power_projection=25 }
		2={ type=army country=1 power_projection=15 }
		3={ type=navy country=1 power_projection=99 }
	}
}
"#,
            crate::empty_tokens(),
        )
        .expect("formation fallback");
        assert_eq!(save.army_power_projection_for(1), Some(40.0));
    }

    #[test]
    fn army_power_unknown_when_no_projection_fields() {
        let save = crate::load_slice(
            br#"SAV01000000000000000000
country_manager={
	database={ 1={ definition="GER" } }
}
armies={
	database={
		1={ type=army country=1 }
	}
}
"#,
            crate::empty_tokens(),
        )
        .expect("no pp fields");
        assert_eq!(save.army_power_projection_for(1), None);
    }

    #[test]
    fn navy_power_falls_back_to_navy_formations() {
        let save = crate::load_slice(
            br#"SAV01000000000000000000
country_manager={
	database={ 1={ definition="GER" } }
}
formation_manager={
	database={
		1={ type=army country=1 power_projection=25 }
		2={ type=navy country=1 power_projection=30 }
	}
}
navy_manager={
	database={
		3={ type=fleet country=1 power_projection=20 }
	}
}
"#,
            crate::empty_tokens(),
        )
        .expect("navy fallback");
        assert_eq!(save.army_power_projection_for(1), Some(25.0));
        assert_eq!(save.navy_power_projection_for(1), Some(50.0));
    }

    #[test]
    fn navy_power_prefers_country_cache() {
        let save = crate::load_slice(
            br#"SAV01000000000000000000
country_manager={
	database={
		1={
			definition="GER"
			cached_total_navy_power_projection=77.5
		}
	}
}
navy_manager={
	database={
		1={ type=navy country=1 power_projection=10 }
	}
}
"#,
            crate::empty_tokens(),
        )
        .expect("navy cache");
        assert_eq!(save.navy_power_projection_for(1), Some(77.5));
    }

    #[test]
    fn world_save_deserializes_navy_manager() {
        let world = crate::load_slice_world(
            br#"SAV01000000000000000000
country_manager={
	database={ 1={ definition="GER" } }
}
navy_manager={
	database={
		1={ type=navy country=1 power_projection=12 }
	}
}
"#,
            crate::empty_tokens(),
        )
        .expect("world save navy");
        assert_eq!(crate::navy_power_projection_for(&world, 1), Some(12.0));
    }
}
