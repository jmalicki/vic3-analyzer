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
    pub wealth: Option<i32>,
    #[serde(default)]
    pub culture: Option<String>,
    #[serde(default, rename = "location", alias = "state")]
    pub state: Option<u32>,
    /// Frozen wage bill when present; omitted pops keep wealth at the saved integer.
    #[serde(default)]
    pub wages: Option<f64>,
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
