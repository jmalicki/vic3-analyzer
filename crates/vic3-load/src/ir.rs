//! Product IR deserialized from a Vic3 save via jomini / `DeserializeVic3`.
//!
//! Field names follow the save. Missing keys default so a small fixture (and
//! older patches) still load. Extra save keys are ignored.

use crate::maybe::maybe_map;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
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
    #[serde(default)]
    pub previous_played: Vec<Player>,
}

/// Save fields the market projection needs.
///
/// Unknown top-level keys (markets, trade routes, construction queues, …) are
/// skipped without allocating IR. That cuts peak RSS and `drop` time; it does
/// **not** skip zlib inflate of a single-member `gamestate` zip. Keep [`Save`]
/// when those extra managers are part of the answer (`parse_save` counts).
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
    #[serde(default)]
    pub previous_played: Vec<Player>,
}

/// Subset of [`Save`] / [`WorldSave`] that [`vic3_prices::World::from_save`] reads.
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
                        SeqTok::Ident(key) if key == "=" => {
                            let key = pending.take().ok_or_else(|| {
                                de::Error::custom("qualification assignment missing key")
                            })?;
                            let qty = seq.next_element::<f64>()?.ok_or_else(|| {
                                de::Error::custom("qualification assignment missing value")
                            })?;
                            values.insert(key, qty);
                        }
                        SeqTok::Ident(key) => pending = Some(key),
                        SeqTok::Float(qty) => {
                            if let Some(key) = pending.take() {
                                values.insert(key, qty);
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
}
