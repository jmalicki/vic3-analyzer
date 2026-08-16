//! Compact [`PlanningState`] projection of a save plus last price solve.
//!
//! This is **not** the full save. Hash it: identical state ⇒ identical hash (I8).

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use vic3_prices::PricesResult;

pub use vic3_load::{Save, Vic3Date};

/// Failure while projecting a [`PlanningState`] from save IR.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WorldError {
    /// No live country whose `definition` matches the requested tag.
    #[error("country tag `{0}` not found in save")]
    UnknownCountry(String),
}

/// Game start used when a save has no `meta_data.game_date`.
pub fn default_date() -> Vic3Date {
    Vic3Date::from_ymdh(1836, 1, 1, 0)
}

/// Inputs for [`PlanningState::from_parts`] (tests; no `.v3` required).
#[derive(Debug, Clone)]
pub struct PlanningParts {
    pub date: Vic3Date,
    pub country: String,
    pub techs: Vec<String>,
    pub good_prices: Vec<(String, f64)>,
    pub solvent: bool,
    pub treasury: f64,
    pub army_power_projection: f64,
    pub interest: Vec<String>,
    pub queued_tech: Option<String>,
    pub gdp: f64,
}

impl Default for PlanningParts {
    fn default() -> Self {
        Self {
            date: default_date(),
            country: String::new(),
            techs: Vec::new(),
            good_prices: Vec::new(),
            solvent: false,
            treasury: 0.0,
            army_power_projection: 0.0,
            interest: Vec::new(),
            queued_tech: None,
            gdp: 0.0,
        }
    }
}

/// Compact world used by the goal DSL and (later) A*.
///
/// Fields are exactly the atoms the compiled goal can read in phase 7a, plus
/// treasury / queued-tech so later successors can wait on payday or research.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningState {
    pub date: Vic3Date,
    /// Country tag (`GER`, `FRA`, …).
    pub country: String,
    /// Researched technology ids.
    pub techs: BTreeSet<String>,
    /// Market prices after the last solve (good id → price).
    pub good_prices: BTreeMap<String, f64>,
    /// Can pay the army without an immediate default (frozen wages/employment).
    pub solvent: bool,
    pub treasury: f64,
    /// Army power projection. Default `0` when the save does not expose it.
    pub army_power_projection: f64,
    /// Strategic regions / states in which the country has declared interest.
    pub interest: BTreeSet<String>,
    /// Technology currently in the research queue, if any.
    pub queued_tech: Option<String>,
    /// Current GDP after prices (model series, not Paradox’s binary).
    pub gdp: f64,
}

impl Default for PlanningState {
    fn default() -> Self {
        Self::from_parts(PlanningParts::default())
    }
}

impl PartialEq for PlanningState {
    fn eq(&self, other: &Self) -> bool {
        self.date == other.date
            && self.country == other.country
            && self.techs == other.techs
            && f64_map_eq(&self.good_prices, &other.good_prices)
            && self.solvent == other.solvent
            && f64_bits_eq(self.treasury, other.treasury)
            && f64_bits_eq(self.army_power_projection, other.army_power_projection)
            && self.interest == other.interest
            && self.queued_tech == other.queued_tech
            && f64_bits_eq(self.gdp, other.gdp)
    }
}

impl Eq for PlanningState {}

impl Hash for PlanningState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.date.hash(state);
        self.country.hash(state);
        self.techs.hash(state);
        self.good_prices.len().hash(state);
        for (k, v) in &self.good_prices {
            k.hash(state);
            v.to_bits().hash(state);
        }
        self.solvent.hash(state);
        self.treasury.to_bits().hash(state);
        self.army_power_projection.to_bits().hash(state);
        self.interest.hash(state);
        self.queued_tech.hash(state);
        self.gdp.to_bits().hash(state);
    }
}

fn f64_bits_eq(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits()
}

fn f64_map_eq(a: &BTreeMap<String, f64>, b: &BTreeMap<String, f64>) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|((ka, va), (kb, vb))| ka == kb && f64_bits_eq(*va, *vb))
}

/// Something that can supply good id → price for [`PlanningState::from_save`].
pub trait IntoPriceMap {
    fn into_price_map(self) -> BTreeMap<String, f64>;
}

impl IntoPriceMap for &PricesResult {
    fn into_price_map(self) -> BTreeMap<String, f64> {
        self.goods.iter().map(|g| (g.id.clone(), g.price)).collect()
    }
}

impl IntoPriceMap for PricesResult {
    fn into_price_map(self) -> BTreeMap<String, f64> {
        (&self).into_price_map()
    }
}

impl IntoPriceMap for BTreeMap<String, f64> {
    fn into_price_map(self) -> BTreeMap<String, f64> {
        self
    }
}

impl IntoPriceMap for &BTreeMap<String, f64> {
    fn into_price_map(self) -> BTreeMap<String, f64> {
        self.clone()
    }
}

impl IntoPriceMap for Vec<(String, f64)> {
    fn into_price_map(self) -> BTreeMap<String, f64> {
        self.into_iter().collect()
    }
}

impl PlanningState {
    /// Fake projection for tests. Does not load a `.v3`.
    pub fn from_parts(parts: PlanningParts) -> Self {
        Self {
            date: parts.date,
            country: parts.country,
            techs: parts.techs.into_iter().collect(),
            good_prices: parts.good_prices.into_iter().collect(),
            solvent: parts.solvent,
            treasury: parts.treasury,
            army_power_projection: parts.army_power_projection,
            interest: parts.interest.into_iter().collect(),
            queued_tech: parts.queued_tech,
            gdp: parts.gdp,
        }
    }

    /// Project the player country and last price solve into a planning node.
    ///
    /// Techs, interest, army power, queued tech, and GDP are not on the current
    /// save IR: they default to empty / `0` / `None`. Fill them with
    /// [`Self::from_parts`] in tests.
    pub fn from_save(
        save: &Save,
        country_tag: &str,
        prices: impl IntoPriceMap,
    ) -> Result<Self, WorldError> {
        let country = save
            .country_by_tag(country_tag)
            .ok_or_else(|| WorldError::UnknownCountry(country_tag.to_string()))?;
        let treasury = country.budget.treasury().unwrap_or(0.0);
        Ok(Self {
            date: save.meta_data.game_date.unwrap_or_else(default_date),
            country: country.definition.clone(),
            techs: BTreeSet::new(),
            good_prices: prices.into_price_map(),
            solvent: treasury > 0.0,
            treasury,
            army_power_projection: 0.0,
            interest: BTreeSet::new(),
            queued_tech: None,
            gdp: 0.0,
        })
    }

    /// I8 fingerprint: identical state ⇒ identical `u64`.
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = std::hash::DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }

    /// Price of `good` after the last solve, if present.
    pub fn price(&self, good: &str) -> Option<f64> {
        self.good_prices.get(good).copied()
    }

    /// Whether `tech` is in the researched set.
    pub fn has_tech(&self, tech: &str) -> bool {
        self.techs.contains(tech)
    }

    /// Whether the country has interest in `id` (state or region as in the DSL).
    pub fn has_interest(&self, id: &str) -> bool {
        self.interest.contains(id)
    }
}

/// Crate version from Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use vic3_load::{Budget, Country, Manager, Meta, Save};
    use vic3_prices::{GoodPrice, PricesResult, SolveStatus};

    fn ger_save(treasury: f64) -> Save {
        let mut countries = Manager::<Country>::default();
        countries.database.insert(
            1,
            Some(Country {
                definition: "GER".into(),
                budget: Budget {
                    gold_reserves: Some(treasury),
                    ..Budget::default()
                },
                ..Country::default()
            }),
        );
        Save {
            meta_data: Meta {
                version: "1.9.0".into(),
                game_date: Some(Vic3Date::from_ymdh(1850, 6, 1, 0)),
                name: Some("Fixture".into()),
            },
            country_manager: countries,
            ..Save::default()
        }
    }

    fn ammo_prices(price: f64) -> PricesResult {
        PricesResult {
            scope: "whole_save_synthetic".into(),
            goods: vec![GoodPrice {
                id: "ammunition".into(),
                name: None,
                base: 50.0,
                price,
                buy: 1.0,
                sell: 1.0,
            }],
            states: Vec::new(),
            state_goods: Vec::new(),
            buildings: Vec::new(),
            residual: 0.0,
            status: SolveStatus::Converged,
            limitations: Vec::new(),
        }
    }

    #[test]
    fn version_is_semver() {
        assert!(!version().is_empty());
    }

    #[test]
    fn from_parts_fake_state_no_save() {
        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            techs: vec!["nitroglycerin".into()],
            good_prices: vec![("ammunition".into(), 32.0)],
            solvent: true,
            treasury: 1_000.0,
            army_power_projection: 120.0,
            interest: vec!["alsace".into()],
            queued_tech: Some("mechanized_agriculture".into()),
            gdp: 50e6,
            ..PlanningParts::default()
        });
        assert_eq!(state.country, "GER");
        assert!(state.has_tech("nitroglycerin"));
        assert_eq!(state.price("ammunition"), Some(32.0));
        assert!(state.solvent);
        assert!(state.has_interest("alsace"));
        assert_eq!(state.queued_tech.as_deref(), Some("mechanized_agriculture"));
        assert_eq!(state.army_power_projection, 120.0);
        assert_eq!(state.gdp, 50e6);
    }

    #[test]
    fn from_save_uses_prices_result_and_treasury() {
        let save = ger_save(10_000.0);
        let state = PlanningState::from_save(&save, "GER", ammo_prices(40.0)).unwrap();
        assert_eq!(state.country, "GER");
        assert_eq!(state.date, Vic3Date::from_ymdh(1850, 6, 1, 0));
        assert_eq!(state.treasury, 10_000.0);
        assert!(state.solvent);
        assert_eq!(state.price("ammunition"), Some(40.0));
        assert_eq!(state.army_power_projection, 0.0);
        assert!(state.techs.is_empty());
        assert!(state.interest.is_empty());
        assert!(state.queued_tech.is_none());
    }

    #[test]
    fn from_save_accepts_price_map() {
        let save = ger_save(-50.0);
        let mut prices = BTreeMap::new();
        prices.insert("grain".into(), 20.0);
        let state = PlanningState::from_save(&save, "GER", prices).unwrap();
        assert!(!state.solvent);
        assert_eq!(state.treasury, -50.0);
        assert_eq!(state.price("grain"), Some(20.0));
    }

    #[test]
    fn from_save_unknown_tag() {
        let save = ger_save(1.0);
        let err = PlanningState::from_save(&save, "FRA", BTreeMap::new()).unwrap_err();
        assert_eq!(err, WorldError::UnknownCountry("FRA".into()));
    }

    #[test]
    fn i8_identical_planning_state_identical_hash() {
        let a = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            techs: vec!["railways".into(), "nitroglycerin".into()],
            good_prices: vec![("wood".into(), 20.0), ("grain".into(), 30.0)],
            solvent: true,
            treasury: 4.0,
            army_power_projection: 10.0,
            interest: vec!["alsace".into()],
            gdp: 1.0,
            ..PlanningParts::default()
        });
        let b = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            techs: vec!["nitroglycerin".into(), "railways".into()],
            good_prices: vec![("grain".into(), 30.0), ("wood".into(), 20.0)],
            solvent: true,
            treasury: 4.0,
            army_power_projection: 10.0,
            interest: vec!["alsace".into()],
            gdp: 1.0,
            ..PlanningParts::default()
        });
        assert_eq!(a, b);
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_eq!(a.fingerprint(), a.clone().fingerprint());

        let mut other = a.clone();
        other.treasury = 5.0;
        assert_ne!(a, other);
        assert_ne!(a.fingerprint(), other.fingerprint());
    }

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        /// I8 for this struct: identical [`PlanningState`] ⇒ identical hash.
        #[test]
        fn i8_clone_round_trip_same_hash(
            country in "[A-Z]{3}",
            treasury in -1_000.0f64..1_000.0,
            army in 0.0f64..500.0,
            solvent in any::<bool>(),
            gdp in 0.0f64..1e9,
            ammo in 1.0f64..100.0,
        ) {
            let state = PlanningState::from_parts(PlanningParts {
                country,
                treasury,
                army_power_projection: army,
                solvent,
                gdp,
                good_prices: vec![("ammunition".into(), ammo)],
                ..PlanningParts::default()
            });
            let clone = state.clone();
            prop_assert_eq!(&state, &clone);
            prop_assert_eq!(state.fingerprint(), clone.fingerprint());
        }
    }
}
