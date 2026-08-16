//! Product IR deserialized from a Vic3 save via jomini / `DeserializeVic3`.
//!
//! Field names follow the save. Missing keys default so a small fixture (and
//! older patches) still load. Extra save keys are ignored.

use crate::maybe::maybe_map;
use serde::Deserialize;
use std::collections::HashMap;
use vic3save::Vic3Date;

/// Paradox `foo_manager.database` (or `states.database`, `pops.database`, …):
/// each id is either an object or `none`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(bound = "T: Deserialize<'de>")]
pub struct Manager<T> {
    #[serde(default, deserialize_with = "maybe_map")]
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
    #[serde(default)]
    pub definition: String,
    #[serde(default)]
    pub government: Option<String>,
    #[serde(default)]
    pub capital: Option<u32>,
    #[serde(default)]
    pub states: Vec<u32>,
    #[serde(default)]
    pub infamy: Option<f64>,
    #[serde(default)]
    pub budget: Budget,
    /// Enacted law type ids when the save records them (`law_autocracy`, …).
    #[serde(default)]
    pub laws: Vec<String>,
    /// Direct overlord country id when this country is a subject.
    #[serde(default)]
    pub overlord: Option<u32>,
    /// Subject type id when present (`puppet`, `dominion`, …).
    #[serde(default)]
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
    #[serde(default)]
    pub credit: Option<f64>,
    #[serde(default)]
    pub weekly_income: Vec<f64>,
}

impl Budget {
    /// Best-effort cash on hand (`gold_reserves`, then `gold`, then `money`).
    pub fn treasury(&self) -> Option<f64> {
        self.gold_reserves.or(self.gold).or(self.money)
    }
}

/// A state in `states.database`.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct State {
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub country: Option<u32>,
    #[serde(default)]
    pub market: Option<u32>,
}

/// A building in `building_manager.database`.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct Building {
    #[serde(default)]
    pub building: String,
    #[serde(default)]
    pub state: Option<u32>,
    #[serde(default)]
    pub level: i32,
    #[serde(default)]
    pub staffing: f64,
    /// Active production method when a save records a single id.
    #[serde(default)]
    pub production_method: Option<String>,
    /// Active production methods, one per PM group. This is the shape a real
    /// save uses; a building runs every listed method at once.
    #[serde(default)]
    pub production_methods: Vec<String>,
    /// Saved input volumes when present. Keys are good script ids (fixtures) or
    /// the vanilla goods-table index as a decimal string (real saves).
    #[serde(default)]
    pub input_goods: BuildingGoods,
    /// Saved output volumes when present.
    #[serde(default)]
    pub output_goods: BuildingGoods,
}

/// Goods quantities on a building (`input_goods` / `output_goods`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BuildingGoods {
    pub goods: std::collections::BTreeMap<String, f64>,
}

impl<'de> Deserialize<'de> for BuildingGoods {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            goods: std::collections::BTreeMap<String, GoodQty>,
        }
        let raw = Raw::deserialize(deserializer).unwrap_or(Raw {
            goods: Default::default(),
        });
        Ok(Self {
            goods: raw
                .goods
                .into_iter()
                .map(|(key, qty)| (resolve_good_key(&key), qty.0))
                .filter(|(_, qty)| *qty != 0.0)
                .collect(),
        })
    }
}

/// Float or `{ value = … }` (Vic3 1.9+).
#[derive(Debug, Clone, PartialEq)]
struct GoodQty(f64);

impl<'de> Deserialize<'de> for GoodQty {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = GoodQty;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a number or { value = … }")
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Ok(GoodQty(v))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(GoodQty(v as f64))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(GoodQty(v as f64))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                v.parse().map(GoodQty).map_err(E::custom)
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut value = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key == "value" {
                        value = Some(map.next_value::<f64>()?);
                    } else {
                        let _: serde::de::IgnoredAny = map.next_value()?;
                    }
                }
                Ok(GoodQty(value.unwrap_or(0.0)))
            }
        }
        deserializer.deserialize_any(V)
    }
}

/// Vanilla `common/goods` order used as the integer index in real saves.
const VANILLA_GOODS: &[&str] = &[
    "ammunition",
    "small_arms",
    "artillery",
    "tanks",
    "aeroplanes",
    "manowars",
    "ironclads",
    "grain",
    "fish",
    "fabric",
    "wood",
    "groceries",
    "clothes",
    "furniture",
    "paper",
    "services",
    "transportation",
    "electricity",
    "clippers",
    "steamers",
    "silk",
    "dye",
    "sulfur",
    "coal",
    "iron",
    "lead",
    "hardwood",
    "rubber",
    "oil",
    "engines",
    "steel",
    "glass",
    "fertilizer",
    "tools",
    "explosives",
    "porcelain",
    "meat",
    "fruit",
    "liquor",
    "wine",
    "tea",
    "coffee",
    "sugar",
    "tobacco",
    "opium",
    "automobiles",
    "telephones",
    "radios",
    "luxury_clothes",
    "luxury_furniture",
    "gold",
    "fine_art",
];

fn resolve_good_key(key: &str) -> String {
    if let Ok(index) = key.parse::<usize>() {
        if let Some(name) = VANILLA_GOODS.get(index) {
            return (*name).to_string();
        }
    }
    key.to_string()
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

/// A pop in `pops.database`.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct Pop {
    #[serde(default, rename = "type", alias = "profession")]
    pub profession: Option<String>,
    #[serde(default)]
    pub size: Option<f64>,
    #[serde(default)]
    pub size_wa: Option<f64>,
    #[serde(default)]
    pub size_dn: Option<f64>,
    #[serde(default)]
    pub wealth: Option<i32>,
    #[serde(default)]
    pub culture: Option<String>,
    #[serde(default, rename = "location", alias = "state")]
    pub state: Option<u32>,
    /// Frozen wage bill when present; omitted pops keep wealth at the saved integer.
    #[serde(default)]
    pub wages: Option<f64>,
}

impl Pop {
    /// Working-adult-equivalent population used to scale demand.
    ///
    /// Current saves split pop size into working adults (`size_wa`) and
    /// dependents (`size_dn`); older saves use the legacy `size` field.
    pub fn demand_size(&self) -> Option<f64> {
        if self.size_wa.is_some() || self.size_dn.is_some() {
            let size = self.size_wa.unwrap_or(0.0) + 0.5 * self.size_dn.unwrap_or(0.0);
            return (size > 0.0).then_some(size);
        }

        self.size.filter(|size| *size > 0.0)
    }
}

/// A market in `market_manager.database`.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct Market {
    #[serde(default)]
    pub states: Vec<u32>,
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
}

#[cfg(test)]
mod tests {
    use super::Pop;

    #[test]
    fn demand_size_weights_dependents() {
        let pop = Pop {
            size_wa: Some(8_000.0),
            size_dn: Some(4_000.0),
            ..Pop::default()
        };

        assert_eq!(pop.demand_size(), Some(10_000.0));
    }

    #[test]
    fn demand_size_falls_back_to_legacy_size() {
        let pop = Pop {
            size: Some(10_000.0),
            ..Pop::default()
        };

        assert_eq!(pop.demand_size(), Some(10_000.0));
    }
}
