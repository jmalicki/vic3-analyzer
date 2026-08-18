//! Compact [`PlanningState`] projection of a save plus last price solve.
//!
//! This is **not** the full save. Hash it: identical state ⇒ identical hash (I8).

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use vic3_prices::{PricesResult, World, WorldCountry};

pub use vic3_load::{Save, Vic3Date};

/// Failure while projecting a [`PlanningState`] from save IR or [`World`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WorldError {
    /// No live country whose `definition` / tag matches the requested tag.
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
    pub weekly_balance: Option<f64>,
    pub population_weighted_wealth: Option<f64>,
    pub debt_principal: Option<f64>,
    pub credit_limit: Option<f64>,
    pub credit_headroom: Option<f64>,
    pub building_level_deltas: BTreeMap<String, u32>,
    pub queued_building: Option<String>,
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
            weekly_balance: None,
            population_weighted_wealth: None,
            debt_principal: None,
            credit_limit: None,
            credit_headroom: None,
            building_level_deltas: BTreeMap::new(),
            queued_building: None,
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
    /// Known remaining credit before exhaustion (`principal < credit`).
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
    /// Most recent finite saved net weekly-budget sample.
    pub weekly_balance: Option<f64>,
    /// Population-weighted saved pop wealth, exposed as an SoL proxy.
    pub population_weighted_wealth: Option<f64>,
    /// Outstanding debt principal when present on the save budget.
    pub debt_principal: Option<f64>,
    /// Credit limit when present on the save budget.
    pub credit_limit: Option<f64>,
    /// Remaining credit (`credit_limit - debt_principal`) when both are known.
    pub credit_headroom: Option<f64>,
    /// Explicit added levels by building type in this simulated branch.
    pub building_level_deltas: BTreeMap<String, u32>,
    /// Building type currently in the compact construction queue.
    pub queued_building: Option<String>,
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
            && f64_option_bits_eq(self.weekly_balance, other.weekly_balance)
            && f64_option_bits_eq(
                self.population_weighted_wealth,
                other.population_weighted_wealth,
            )
            && f64_option_bits_eq(self.debt_principal, other.debt_principal)
            && f64_option_bits_eq(self.credit_limit, other.credit_limit)
            && f64_option_bits_eq(self.credit_headroom, other.credit_headroom)
            && self.building_level_deltas == other.building_level_deltas
            && self.queued_building == other.queued_building
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
        hash_f64_option(self.weekly_balance, state);
        hash_f64_option(self.population_weighted_wealth, state);
        hash_f64_option(self.debt_principal, state);
        hash_f64_option(self.credit_limit, state);
        hash_f64_option(self.credit_headroom, state);
        self.building_level_deltas.hash(state);
        self.queued_building.hash(state);
    }
}

fn f64_bits_eq(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits()
}

fn f64_option_bits_eq(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => f64_bits_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

fn hash_f64_option<H: Hasher>(value: Option<f64>, state: &mut H) {
    value.is_some().hash(state);
    if let Some(value) = value {
        value.to_bits().hash(state);
    }
}

fn f64_map_eq(a: &BTreeMap<String, f64>, b: &BTreeMap<String, f64>) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|((ka, va), (kb, vb))| ka == kb && f64_bits_eq(*va, *vb))
}

/// Something that can supply good id → price for [`PlanningState::from_save`]
/// / [`PlanningState::from_world`].
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
            weekly_balance: parts.weekly_balance,
            population_weighted_wealth: parts.population_weighted_wealth,
            debt_principal: parts.debt_principal,
            credit_limit: parts.credit_limit,
            credit_headroom: parts.credit_headroom,
            building_level_deltas: parts.building_level_deltas,
            queued_building: parts.queued_building,
        }
    }

    /// Project the player country and last price solve into a planning node.
    ///
    /// Techs, interest, army power, queued tech, and GDP are not on the current
    /// save IR: they default to empty / `0` / `None`. Weekly balance is the last
    /// saved sample; population-weighted wealth is computed from pops in states
    /// owned by this country. Solvency requires known principal and credit.
    /// Missing metrics remain `None`.
    pub fn from_save(
        save: &Save,
        country_tag: &str,
        prices: impl IntoPriceMap,
    ) -> Result<Self, WorldError> {
        let (country_id, country) = save
            .countries()
            .find(|(_, country)| country.definition == country_tag)
            .ok_or_else(|| WorldError::UnknownCountry(country_tag.to_string()))?;
        let treasury = country.budget.treasury().unwrap_or(0.0);
        let weekly_balance = country
            .budget
            .weekly_income
            .last()
            .copied()
            .filter(|value| value.is_finite());
        let debt_principal = country.budget.principal.filter(|value| value.is_finite());
        let credit_limit = country.budget.credit.filter(|value| value.is_finite());
        let credit_headroom = country.budget.credit_headroom();
        let mut owned_states: BTreeSet<u32> = save
            .states
            .iter_present()
            .filter_map(|(id, state)| (state.country == Some(country_id)).then_some(id))
            .collect();
        if owned_states.is_empty() {
            owned_states.extend(country.states.iter().copied());
        }
        let (weighted_sol, population) = save
            .pops
            .iter_present()
            .filter_map(|(_, pop)| {
                let state = pop.state?;
                if !owned_states.contains(&state) {
                    return None;
                }
                let size = pop.demand_size()?;
                let wealth = f64::from(pop.wealth?);
                Some((wealth * size, size))
            })
            .fold(
                (0.0, 0.0),
                |(wealth, population), (next_wealth, next_population)| {
                    (wealth + next_wealth, population + next_population)
                },
            );
        let population_weighted_wealth = if population > 0.0 {
            Some(weighted_sol / population)
        } else {
            None
        };
        Ok(Self {
            date: save.meta_data.game_date.unwrap_or_else(default_date),
            country: country.definition.clone(),
            techs: BTreeSet::new(),
            good_prices: prices.into_price_map(),
            solvent: country.budget.is_solvent(),
            treasury,
            army_power_projection: 0.0,
            interest: BTreeSet::new(),
            queued_tech: None,
            gdp: 0.0,
            weekly_balance,
            population_weighted_wealth,
            debt_principal,
            credit_limit,
            credit_headroom,
            building_level_deltas: BTreeMap::new(),
            queued_building: None,
        })
    }

    /// Project from a full price result, including modeled GDP.
    ///
    /// GDP is gross building output value (`revenue`) in states owned by the
    /// selected country under the same solved prices.
    pub fn from_save_with_prices(
        save: &Save,
        country_tag: &str,
        prices: &PricesResult,
    ) -> Result<Self, WorldError> {
        let (country_id, country) = save
            .countries()
            .find(|(_, country)| country.definition == country_tag)
            .ok_or_else(|| WorldError::UnknownCountry(country_tag.to_string()))?;
        let mut owned_states: BTreeSet<u32> = save
            .states
            .iter_present()
            .filter_map(|(id, state)| (state.country == Some(country_id)).then_some(id))
            .collect();
        if owned_states.is_empty() {
            owned_states.extend(country.states.iter().copied());
        }
        let mut state = Self::from_save(save, country_tag, prices)?;
        state.gdp = prices
            .buildings
            .iter()
            .filter(|building| {
                building
                    .state_id
                    .is_some_and(|state_id| owned_states.contains(&state_id))
            })
            .map(|building| building.revenue.max(0.0))
            .sum();
        Ok(state)
    }

    /// Project the player country from a compact [`World`] plus last prices.
    ///
    /// Techs, interest, army power, queued tech, and GDP default to empty / `0`
    /// / `None`. Population-weighted wealth uses pops in states owned by this
    /// country. Solvency and budget lines come from [`WorldCountry`].
    pub fn from_world(
        world: &World,
        country_tag: &str,
        prices: impl IntoPriceMap,
    ) -> Result<Self, WorldError> {
        let country = world
            .country_by_tag(country_tag)
            .ok_or_else(|| WorldError::UnknownCountry(country_tag.to_string()))?;
        Ok(Self {
            date: world.game_date.unwrap_or_else(default_date),
            country: country.tag.clone(),
            techs: BTreeSet::new(),
            good_prices: prices.into_price_map(),
            solvent: country.solvent,
            treasury: country.treasury,
            army_power_projection: 0.0,
            interest: BTreeSet::new(),
            queued_tech: None,
            gdp: 0.0,
            weekly_balance: country.weekly_balance,
            population_weighted_wealth: population_weighted_wealth(world, country),
            debt_principal: country.debt_principal,
            credit_limit: country.credit_limit,
            credit_headroom: country.credit_headroom,
            building_level_deltas: BTreeMap::new(),
            queued_building: None,
        })
    }

    /// Project from a full price result, including modeled GDP.
    ///
    /// GDP is gross building output value (`revenue`) in states owned by the
    /// selected country under the same solved prices.
    pub fn from_world_with_prices(
        world: &World,
        country_tag: &str,
        prices: &PricesResult,
    ) -> Result<Self, WorldError> {
        let country = world
            .country_by_tag(country_tag)
            .ok_or_else(|| WorldError::UnknownCountry(country_tag.to_string()))?;
        let owned_states = owned_state_ids(world, country);
        let mut state = Self::from_world(world, country_tag, prices)?;
        state.gdp = prices
            .buildings
            .iter()
            .filter(|building| {
                building
                    .state_id
                    .is_some_and(|state_id| owned_states.contains(&state_id))
            })
            .map(|building| building.revenue.max(0.0))
            .sum();
        Ok(state)
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

fn owned_state_ids(world: &World, country: &WorldCountry) -> BTreeSet<u32> {
    let mut owned_states: BTreeSet<u32> = world
        .states
        .iter()
        .filter_map(|state| (state.country == Some(country.id)).then_some(state.id))
        .collect();
    if owned_states.is_empty() {
        owned_states.extend(country.states.iter().copied());
    }
    owned_states
}

fn population_weighted_wealth(world: &World, country: &WorldCountry) -> Option<f64> {
    let owned_states = owned_state_ids(world, country);
    let (weighted_sol, population) = if world.state_pops.is_empty() {
        world
            .pops
            .iter()
            .filter_map(|pop| {
                let state = pop.state?;
                if !owned_states.contains(&state) {
                    return None;
                }
                Some((f64::from(pop.wealth) * pop.size, pop.size))
            })
            .fold(
                (0.0, 0.0),
                |(wealth, population), (next_wealth, next_population)| {
                    (wealth + next_wealth, population + next_population)
                },
            )
    } else {
        world
            .state_pops
            .iter()
            .filter_map(|pop| {
                let state = pop.state?;
                if !owned_states.contains(&state) {
                    return None;
                }
                let size = pop.demand_size?;
                let wealth = f64::from(pop.wealth?);
                Some((wealth * size, size))
            })
            .fold(
                (0.0, 0.0),
                |(wealth, population), (next_wealth, next_population)| {
                    (wealth + next_wealth, population + next_population)
                },
            )
    };
    (population > 0.0).then_some(weighted_sol / population)
}

/// Crate version from Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use vic3_load::{Budget, Country, Manager, Meta, Pop, Save, State};
    use vic3_prices::{BuildingEconomics, GoodPrice, PricesResult, SolveStatus, World};

    fn ger_save(treasury: f64) -> Save {
        ger_budget_save(
            treasury,
            Budget {
                gold_reserves: Some(treasury),
                credit: Some(500.0),
                principal: Some(0.0),
                weekly_income: vec![50.0, 100.0],
                ..Budget::default()
            },
        )
    }

    fn ger_budget_save(treasury: f64, budget: Budget) -> Save {
        let mut countries = Manager::<Country>::default();
        countries.database.insert(
            1,
            Some(Country {
                definition: "GER".into(),
                budget: Budget {
                    gold_reserves: budget.gold_reserves.or(Some(treasury)),
                    ..budget
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
            building_types: Vec::new(),
            building_groups: Vec::new(),
            state_pops: Vec::new().into(),
            countries: Vec::new(),
            inputs: Default::default(),
            residual: 0.0,
            status: SolveStatus::Converged,
            limitations: Vec::new(),
            state_qualifications: Vec::new(),
            state_needs: Vec::new(),
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
        assert_eq!(state.weekly_balance, Some(100.0));
        assert_eq!(state.debt_principal, Some(0.0));
        assert_eq!(state.credit_limit, Some(500.0));
        assert_eq!(state.credit_headroom, Some(500.0));
        assert_eq!(state.price("ammunition"), Some(40.0));
        assert_eq!(state.army_power_projection, 0.0);
        assert!(state.techs.is_empty());
        assert!(state.interest.is_empty());
        assert!(state.queued_tech.is_none());
    }

    #[test]
    fn from_save_solvent_uses_credit_headroom_not_treasury_sign() {
        let indebted = ger_budget_save(
            -50.0,
            Budget {
                gold_reserves: Some(-50.0),
                credit: Some(1_000.0),
                principal: Some(200.0),
                weekly_income: vec![10.0],
                ..Budget::default()
            },
        );
        let indebted_state = PlanningState::from_save(&indebted, "GER", BTreeMap::new()).unwrap();
        assert!(indebted_state.solvent);
        assert_eq!(indebted_state.credit_headroom, Some(800.0));

        let exhausted = ger_budget_save(
            1_000.0,
            Budget {
                gold_reserves: Some(1_000.0),
                credit: Some(500.0),
                principal: Some(500.0),
                ..Budget::default()
            },
        );
        let exhausted_state = PlanningState::from_save(&exhausted, "GER", BTreeMap::new()).unwrap();
        assert!(!exhausted_state.solvent);
        assert_eq!(exhausted_state.credit_headroom, Some(0.0));

        let unknown = ger_budget_save(
            1_000.0,
            Budget {
                gold_reserves: Some(1_000.0),
                ..Budget::default()
            },
        );
        let unknown_state = PlanningState::from_save(&unknown, "GER", BTreeMap::new()).unwrap();
        assert!(!unknown_state.solvent);
        assert_eq!(unknown_state.credit_headroom, None);
    }

    #[test]
    fn from_save_population_weights_wealth_for_owned_states() {
        let mut save = ger_save(10_000.0);
        save.country_manager
            .database
            .get_mut(&1)
            .unwrap()
            .as_mut()
            .unwrap()
            .states = vec![20];
        save.states.database.insert(
            10,
            Some(State {
                country: Some(1),
                ..State::default()
            }),
        );
        save.states.database.insert(
            20,
            Some(State {
                country: Some(2),
                ..State::default()
            }),
        );
        save.pops.database.insert(
            1,
            Some(Pop {
                size: Some(1_000.0),
                wealth: Some(10),
                state: Some(10),
                ..Pop::default()
            }),
        );
        save.pops.database.insert(
            2,
            Some(Pop {
                size: Some(3_000.0),
                wealth: Some(20),
                state: Some(10),
                ..Pop::default()
            }),
        );
        save.pops.database.insert(
            3,
            Some(Pop {
                size: Some(10_000.0),
                wealth: Some(99),
                state: Some(20),
                ..Pop::default()
            }),
        );

        let state = PlanningState::from_save(&save, "GER", BTreeMap::new()).unwrap();
        assert_eq!(state.population_weighted_wealth, Some(17.5));
    }

    #[test]
    fn from_save_with_prices_models_gdp_from_owned_building_revenue() {
        let mut save = ger_save(10_000.0);
        save.states.database.insert(
            10,
            Some(State {
                country: Some(1),
                ..State::default()
            }),
        );
        save.states.database.insert(
            20,
            Some(State {
                country: Some(2),
                ..State::default()
            }),
        );
        let mut prices = ammo_prices(40.0);
        let building = |id, state_id, revenue| BuildingEconomics {
            id,
            state_id: Some(state_id),
            type_id: "building_factory".into(),
            level: 1.0,
            staffing: 1.0,
            production_method_ids: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            revenue,
            cost: 0.0,
            profit: revenue,
            short_inputs: Vec::new(),
            employees: Vec::new(),
        };
        prices.buildings = vec![building(1, 10, 125.0), building(2, 20, 900.0)];

        let state = PlanningState::from_save_with_prices(&save, "GER", &prices).unwrap();
        assert_eq!(state.gdp, 125.0);
    }

    #[test]
    fn from_save_accepts_price_map() {
        let save = ger_save(-50.0);
        let mut prices = BTreeMap::new();
        prices.insert("grain".into(), 20.0);
        let state = PlanningState::from_save(&save, "GER", prices).unwrap();
        assert!(state.solvent);
        assert_eq!(state.treasury, -50.0);
        assert_eq!(state.credit_headroom, Some(500.0));
        assert_eq!(state.price("grain"), Some(20.0));
        assert_eq!(state.population_weighted_wealth, None);
    }

    #[test]
    fn from_save_unknown_tag() {
        let save = ger_save(1.0);
        let err = PlanningState::from_save(&save, "FRA", BTreeMap::new()).unwrap_err();
        assert_eq!(err, WorldError::UnknownCountry("FRA".into()));
    }

    #[test]
    fn from_world_matches_from_save_budget_and_date() {
        let save = ger_save(10_000.0);
        let world = World::from_save(&save, &vic3_defs::GameDefs::default());
        let from_save = PlanningState::from_save(&save, "GER", ammo_prices(40.0)).unwrap();
        let from_world = PlanningState::from_world(&world, "GER", ammo_prices(40.0)).unwrap();
        assert_eq!(from_save, from_world);
        assert_eq!(world.player_country_tag(), Some("GER"));
        assert_eq!(world.game_date, Some(Vic3Date::from_ymdh(1850, 6, 1, 0)));
    }

    #[test]
    fn from_world_population_weights_wealth_for_owned_states() {
        let mut save = ger_save(10_000.0);
        save.country_manager
            .database
            .get_mut(&1)
            .unwrap()
            .as_mut()
            .unwrap()
            .states = vec![20];
        save.states.database.insert(
            10,
            Some(State {
                country: Some(1),
                ..State::default()
            }),
        );
        save.states.database.insert(
            20,
            Some(State {
                country: Some(2),
                ..State::default()
            }),
        );
        save.pops.database.insert(
            1,
            Some(Pop {
                size: Some(1_000.0),
                wealth: Some(10),
                state: Some(10),
                ..Pop::default()
            }),
        );
        save.pops.database.insert(
            2,
            Some(Pop {
                size: Some(3_000.0),
                wealth: Some(20),
                state: Some(10),
                ..Pop::default()
            }),
        );
        save.pops.database.insert(
            3,
            Some(Pop {
                size: Some(10_000.0),
                wealth: Some(99),
                state: Some(20),
                ..Pop::default()
            }),
        );

        let world = World::from_save(&save, &vic3_defs::GameDefs::default());
        let state = PlanningState::from_world(&world, "GER", BTreeMap::new()).unwrap();
        assert_eq!(state.population_weighted_wealth, Some(17.5));
    }

    #[test]
    fn from_world_with_prices_models_gdp_from_owned_building_revenue() {
        let mut save = ger_save(10_000.0);
        save.states.database.insert(
            10,
            Some(State {
                country: Some(1),
                ..State::default()
            }),
        );
        save.states.database.insert(
            20,
            Some(State {
                country: Some(2),
                ..State::default()
            }),
        );
        let mut prices = ammo_prices(40.0);
        let building = |id, state_id, revenue| BuildingEconomics {
            id,
            state_id: Some(state_id),
            type_id: "building_factory".into(),
            level: 1.0,
            staffing: 1.0,
            production_method_ids: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            revenue,
            cost: 0.0,
            profit: revenue,
            short_inputs: Vec::new(),
            employees: Vec::new(),
        };
        prices.buildings = vec![building(1, 10, 125.0), building(2, 20, 900.0)];

        let world = World::from_save(&save, &vic3_defs::GameDefs::default());
        let state = PlanningState::from_world_with_prices(&world, "GER", &prices).unwrap();
        assert_eq!(state.gdp, 125.0);
    }

    #[test]
    fn from_world_unknown_tag() {
        let save = ger_save(1.0);
        let world = World::from_save(&save, &vic3_defs::GameDefs::default());
        let err = PlanningState::from_world(&world, "FRA", BTreeMap::new()).unwrap_err();
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
