//! Goal DSL: parse → compile sugar → evaluate / list gaps.
//!
//! # Pipeline
//!
//! 1. [`compile`] / [`parse`] — chumsky grammar ([`docs/dsl.md`](../../../docs/dsl.md))
//! 2. Sugar expands to [`SimpleSubgoal`] trees (`declare-war`, `research`, `gdp`)
//! 3. [`evaluate`] / [`gaps`] — read [`crate::world::PlanningState`] only
//!
//! This module does **not** search. Timelines live in `crate::plan`; successors in
//! `crate::sim`.
//!
//! # Sugar
//!
//! | Input | Compiles to |
//! | --- | --- |
//! | `declare-war(state=…)` or `region=` | interest ∧ army ≥ [`DECLARE_WAR_ARMY_THRESHOLD`] ∧ munitions ≤ [`DECLARE_WAR_MUNITIONS_PRICE_CEILING`] ∧ solvent (I-declare-war) |
//! | `colonize(state=…)` or `region=` | colonization tech ∧ colonial law ∨ ∧ quinine ∧ interest ∧ army ≥ [`COLONIZE_ARMY_THRESHOLD`] ∧ navy ≥ [`COLONIZE_NAVY_THRESHOLD`] ∧ solvent (I-colonize) |
//! | `research(tech=…)` / `has_tech(…)` | [`SimpleSubgoal::HasTech`] |
//! | `gdp rel n` | [`SimpleSubgoal::Gdp`] on modeled GDP |
//!
//! Optional `tag=` / `wargoal=` on `declare-war` parse but are ignored.
//!
//! # Consumers (same simple subgoals)
//!
//! - Web UI presets (`web/src/planTemplates.ts`) emit ordinary DSL strings
//! - SQL TVFs `plan(goal)` / `gaps(goal)` compile the same string against the
//!   bound session (`vic3-sql`)
//! - CLI / wasm / Tauri call [`evaluate`] / [`gaps`] or A* via `vic3-api`
//!
//! # Errors
//!
//! [`GoalError::Parse`], [`GoalError::DeclareWarTarget`] / [`GoalError::ColonizeTarget`]
//! (no `state=`/`region=`), [`GoalError::ResearchTech`] (missing tech id).

mod parse;

use crate::world::PlanningState;
use serde::{Deserialize, Serialize};

pub use parse::parse;

/// Army power projection required to start a diplomatic play (model threshold).
///
/// Matches the DSL example `army_power_projection >= 100`. Later phases may
/// load this from defs / opts without removing the army conjunct.
pub const DECLARE_WAR_ARMY_THRESHOLD: f64 = 100.0;

/// Ceiling on ammunition price to start a play (model, not Paradox’s binary).
///
/// Matches `good_price(ammunition) <= 40`. Related mil goods may be added later
/// without removing this munitions-price conjunct.
pub const DECLARE_WAR_MUNITIONS_PRICE_CEILING: f64 = 40.0;

/// Good id used as the munitions-price simple subgoal when compiling `declare-war`.
pub const MUNITIONS_GOOD: &str = "ammunition";

/// Colonization society tech id (model / Paradox script key).
pub const COLONIZE_TECH: &str = "colonization";
/// Quinine tech — gates severe-malaria colonization (model / Paradox script key).
pub const COLONIZE_QUININE_TECH: &str = "quinine";
/// Colonial resettlement law id.
pub const LAW_COLONIAL_RESETTLEMENT: &str = "law_colonial_resettlement";
/// Colonial exploitation law id.
pub const LAW_COLONIAL_EXPLOITATION: &str = "law_colonial_exploitation";
/// Army PP threshold compiled into `colonize(...)`.
pub const COLONIZE_ARMY_THRESHOLD: f64 = 100.0;
/// Navy PP threshold compiled into `colonize(...)`.
pub const COLONIZE_NAVY_THRESHOLD: f64 = 100.0;

/// Parse / compile failure (no partial [`Goal`]).
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum GoalError {
    /// Chumsky / unknown-predicate failure.
    #[error("failed to parse goal: {0}")]
    Parse(String),
    /// `declare-war` / bare `interest_in` without `state=` or `region=`.
    #[error("declare-war requires state= or region=")]
    DeclareWarTarget,
    /// `colonize` without `state=` or `region=`.
    #[error("colonize requires state= or region=")]
    ColonizeTarget,
    /// `research` / `has_tech` missing a tech id.
    #[error("research requires tech=")]
    ResearchTech,
}

/// Comparison in `ident rel number` atoms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Rel {
    Le,
    Ge,
    Lt,
    Gt,
    Eq,
}

impl Rel {
    pub fn holds(self, lhs: f64, rhs: f64) -> bool {
        match self {
            Rel::Le => lhs <= rhs,
            Rel::Ge => lhs >= rhs,
            Rel::Lt => lhs < rhs,
            Rel::Gt => lhs > rhs,
            Rel::Eq => (lhs - rhs).abs() <= 1e-9 * (1.0 + lhs.abs()),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Rel::Le => "<=",
            Rel::Ge => ">=",
            Rel::Lt => "<",
            Rel::Gt => ">",
            Rel::Eq => "==",
        }
    }
}

/// Where [`SimpleSubgoal::InterestIn`] points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum InterestKind {
    State,
    Region,
}

/// Cleared / failing / unknown for SQL `gaps()` and agent-facing honesty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimpleSubgoalStatus {
    Cleared,
    Failing,
    /// Metric missing from save IR — not a measured shortfall.
    Unknown,
}

impl SimpleSubgoalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SimpleSubgoalStatus::Cleared => "cleared",
            SimpleSubgoalStatus::Failing => "failing",
            SimpleSubgoalStatus::Unknown => "unknown",
        }
    }
}

/// **Simple subgoal** — a goal node with no further goal children in the
/// compiled tree (may be refined by future sugar).
///
/// Gaps list these; sim successors branch only on open ones; SQL `gaps()`
/// formats them as `predicate` / `status` / `detail` rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SimpleSubgoal {
    HasTech(String),
    HasLaw(String),
    GoodPrice { good: String, rel: Rel, value: f64 },
    ArmyPower { rel: Rel, value: f64 },
    NavyPower { rel: Rel, value: f64 },
    Solvent,
    InterestIn { kind: InterestKind, id: String },
    Gdp { rel: Rel, value: f64 },
    WeeklyBalance { rel: Rel, value: f64 },
    PopulationWeightedWealth { rel: Rel, value: f64 },
    DebtPrincipal { rel: Rel, value: f64 },
    CreditHeadroom { rel: Rel, value: f64 },
}

impl SimpleSubgoal {
    pub fn is_interest(&self) -> bool {
        matches!(self, SimpleSubgoal::InterestIn { .. })
    }

    pub fn is_army(&self) -> bool {
        matches!(self, SimpleSubgoal::ArmyPower { .. })
    }

    pub fn is_navy(&self) -> bool {
        matches!(self, SimpleSubgoal::NavyPower { .. })
    }

    pub fn is_munitions_price(&self) -> bool {
        matches!(self, SimpleSubgoal::GoodPrice { good, .. } if good == MUNITIONS_GOOD)
    }

    pub fn is_solvent(&self) -> bool {
        matches!(self, SimpleSubgoal::Solvent)
    }

    pub fn is_has_tech(&self, tech: &str) -> bool {
        matches!(self, SimpleSubgoal::HasTech(t) if t == tech)
    }

    pub fn is_has_law(&self, law: &str) -> bool {
        matches!(self, SimpleSubgoal::HasLaw(id) if crate::world::law_key(id) == crate::world::law_key(law))
    }

    /// Whether the simple subgoal holds. Unknown metrics (including missing army PP) are false.
    pub fn eval(&self, state: &PlanningState) -> bool {
        matches!(self.status(state), SimpleSubgoalStatus::Cleared)
    }

    /// Cleared / failing / unknown — used by SQL `gaps()` so missing PP is not “failing”.
    pub fn status(&self, state: &PlanningState) -> SimpleSubgoalStatus {
        match self {
            SimpleSubgoal::HasTech(tech) => {
                if state.has_tech(tech) {
                    SimpleSubgoalStatus::Cleared
                } else {
                    SimpleSubgoalStatus::Failing
                }
            }
            SimpleSubgoal::HasLaw(law) => {
                if state.has_law(law) {
                    SimpleSubgoalStatus::Cleared
                } else {
                    SimpleSubgoalStatus::Failing
                }
            }
            SimpleSubgoal::GoodPrice { good, rel, value } => match state.price(good) {
                Some(p) if rel.holds(p, *value) => SimpleSubgoalStatus::Cleared,
                Some(_) => SimpleSubgoalStatus::Failing,
                None => SimpleSubgoalStatus::Unknown,
            },
            SimpleSubgoal::ArmyPower { rel, value } => match state.army_power_projection {
                Some(power) if rel.holds(power, *value) && state.army_buildings_fully_staffed() => {
                    SimpleSubgoalStatus::Cleared
                }
                Some(_) => SimpleSubgoalStatus::Failing,
                None => SimpleSubgoalStatus::Unknown,
            },
            SimpleSubgoal::NavyPower { rel, value } => match state.navy_power_projection {
                Some(power) if rel.holds(power, *value) && state.navy_buildings_fully_staffed() => {
                    SimpleSubgoalStatus::Cleared
                }
                Some(_) => SimpleSubgoalStatus::Failing,
                None => SimpleSubgoalStatus::Unknown,
            },
            SimpleSubgoal::Solvent => {
                if state.solvent {
                    SimpleSubgoalStatus::Cleared
                } else {
                    SimpleSubgoalStatus::Failing
                }
            }
            SimpleSubgoal::InterestIn {
                kind: InterestKind::State,
                id,
            } => {
                if state.has_interest_state(id) {
                    SimpleSubgoalStatus::Cleared
                } else {
                    SimpleSubgoalStatus::Failing
                }
            }
            SimpleSubgoal::InterestIn {
                kind: InterestKind::Region,
                id,
            } => {
                if state.has_interest_region(id) {
                    SimpleSubgoalStatus::Cleared
                } else {
                    SimpleSubgoalStatus::Failing
                }
            }
            SimpleSubgoal::Gdp { rel, value } => {
                if rel.holds(state.gdp, *value) {
                    SimpleSubgoalStatus::Cleared
                } else {
                    SimpleSubgoalStatus::Failing
                }
            }
            SimpleSubgoal::WeeklyBalance { rel, value } => match state.weekly_balance {
                Some(balance) if rel.holds(balance, *value) => SimpleSubgoalStatus::Cleared,
                Some(_) => SimpleSubgoalStatus::Failing,
                None => SimpleSubgoalStatus::Unknown,
            },
            SimpleSubgoal::PopulationWeightedWealth { rel, value } => {
                match state.population_weighted_wealth {
                    Some(wealth) if rel.holds(wealth, *value) => SimpleSubgoalStatus::Cleared,
                    Some(_) => SimpleSubgoalStatus::Failing,
                    None => SimpleSubgoalStatus::Unknown,
                }
            }
            SimpleSubgoal::DebtPrincipal { rel, value } => match state.debt_principal {
                Some(principal) if rel.holds(principal, *value) => SimpleSubgoalStatus::Cleared,
                Some(_) => SimpleSubgoalStatus::Failing,
                None => SimpleSubgoalStatus::Unknown,
            },
            SimpleSubgoal::CreditHeadroom { rel, value } => match state.credit_headroom {
                Some(headroom) if rel.holds(headroom, *value) => SimpleSubgoalStatus::Cleared,
                Some(_) => SimpleSubgoalStatus::Failing,
                None => SimpleSubgoalStatus::Unknown,
            },
        }
    }
}

/// Compiled boolean formula over a [`PlanningState`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Goal {
    And(Vec<Goal>),
    Or(Vec<Goal>),
    Not(Box<Goal>),
    Simple(SimpleSubgoal),
}

impl Goal {
    pub(crate) fn from_and(mut xs: Vec<Goal>) -> Self {
        match xs.len() {
            0 => Goal::And(xs),
            1 => xs.pop().unwrap(),
            _ => Goal::And(xs),
        }
    }

    pub(crate) fn from_or(mut xs: Vec<Goal>) -> Self {
        match xs.len() {
            0 => Goal::Or(xs),
            1 => xs.pop().unwrap(),
            _ => Goal::Or(xs),
        }
    }

    /// Flattened **simple subgoal**s (including under `not`).
    pub fn simple_subgoals(&self) -> Vec<&SimpleSubgoal> {
        let mut out = Vec::new();
        self.collect_simple_subgoals(&mut out);
        out
    }

    fn collect_simple_subgoals<'a>(&'a self, out: &mut Vec<&'a SimpleSubgoal>) {
        match self {
            Goal::And(xs) | Goal::Or(xs) => {
                for x in xs {
                    x.collect_simple_subgoals(out);
                }
            }
            Goal::Not(inner) => inner.collect_simple_subgoals(out),
            Goal::Simple(atom) => out.push(atom),
        }
    }

    pub fn has_interest_simple_subgoal(&self) -> bool {
        self.simple_subgoals().iter().any(|a| a.is_interest())
    }

    pub fn has_army_simple_subgoal(&self) -> bool {
        self.simple_subgoals().iter().any(|a| a.is_army())
    }

    pub fn has_navy_simple_subgoal(&self) -> bool {
        self.simple_subgoals().iter().any(|a| a.is_navy())
    }

    pub fn has_munitions_price_simple_subgoal(&self) -> bool {
        self.simple_subgoals()
            .iter()
            .any(|a| a.is_munitions_price())
    }

    pub fn has_solvent_simple_subgoal(&self) -> bool {
        self.simple_subgoals().iter().any(|a| a.is_solvent())
    }
}

/// Parse the DSL and compile sugar (`declare-war`, `research`, `gdp`).
///
/// Alias of [`parse`]. Prefer this name at API boundaries (CLI, SQL, wasm).
pub fn compile(src: &str) -> Result<Goal, GoalError> {
    parse(src)
}

/// Whether the compiled formula holds on `state`. Does not search.
pub fn evaluate(goal: &Goal, state: &PlanningState) -> bool {
    match goal {
        Goal::And(xs) => xs.iter().all(|g| evaluate(g, state)),
        Goal::Or(xs) => xs.iter().any(|g| evaluate(g, state)),
        Goal::Not(inner) => !evaluate(inner, state),
        Goal::Simple(atom) => atom.eval(state),
    }
}

/// Unsatisfied atoms (empty when [`evaluate`] is true). Does not search.
///
/// Under `And`/`Or`, only failing subtrees contribute; under `Not`, all atoms
/// of the negated subtree are listed when the `not` itself fails.
///
/// Tech gaps are leaf-only here. Prefer [`gaps_with_defs`] when
/// [`vic3_defs::GameDefs`] technologies are loaded so missing ancestors appear.
pub fn gaps(goal: &Goal, state: &PlanningState) -> Vec<SimpleSubgoal> {
    let mut out = Vec::new();
    collect_gaps(goal, state, &mut out);
    out
}

/// Like [`gaps`], then expand each open [`SimpleSubgoal::HasTech`] through prerequisite
/// closure using `defs` (DSL string / compiled goal stay leaf-shaped).
pub fn gaps_with_defs(
    goal: &Goal,
    state: &PlanningState,
    defs: &vic3_defs::GameDefs,
) -> Vec<SimpleSubgoal> {
    let open = gaps(goal, state);
    crate::tech::expand_tech_gap_simple_subgoals(&open, state, defs)
}

fn collect_gaps(goal: &Goal, state: &PlanningState, out: &mut Vec<SimpleSubgoal>) {
    if evaluate(goal, state) {
        return;
    }
    match goal {
        Goal::And(xs) | Goal::Or(xs) => {
            for x in xs {
                collect_gaps(x, state, out);
            }
        }
        Goal::Not(inner) => {
            flatten_simple_subgoals(inner, out);
        }
        Goal::Simple(atom) => out.push(atom.clone()),
    }
}

fn flatten_simple_subgoals(goal: &Goal, out: &mut Vec<SimpleSubgoal>) {
    match goal {
        Goal::And(xs) | Goal::Or(xs) => {
            for x in xs {
                flatten_simple_subgoals(x, out);
            }
        }
        Goal::Not(inner) => flatten_simple_subgoals(inner, out),
        Goal::Simple(atom) => out.push(atom.clone()),
    }
}

/// Crate version from Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{PlanningParts, PlanningState};
    use proptest::prelude::*;

    #[test]
    fn version_is_semver() {
        assert!(!version().is_empty());
    }

    #[test]
    fn golden_declare_war_includes_interest_army_munitions_price_solvent() {
        let goal = parse("declare-war(tag=FRA, wargoal=conquer_state, state=alsace)").unwrap();
        assert!(goal.has_interest_simple_subgoal(), "interest");
        assert!(goal.has_army_simple_subgoal(), "army");
        assert!(goal.has_munitions_price_simple_subgoal(), "munitions-price");
        assert!(goal.has_solvent_simple_subgoal(), "solvent");
        let interest = goal
            .simple_subgoals()
            .into_iter()
            .find(|a| a.is_interest())
            .expect("interest simple subgoal");
        assert_eq!(
            interest,
            &SimpleSubgoal::InterestIn {
                kind: InterestKind::State,
                id: "alsace".into(),
            }
        );
        assert!(goal.simple_subgoals().iter().any(|a| matches!(
            a,
            SimpleSubgoal::ArmyPower {
                rel: Rel::Ge,
                value,
            } if *value == DECLARE_WAR_ARMY_THRESHOLD
        )));
        assert!(goal.simple_subgoals().iter().any(|a| matches!(
            a,
            SimpleSubgoal::GoodPrice {
                good,
                rel: Rel::Le,
                value,
            } if good == MUNITIONS_GOOD && *value == DECLARE_WAR_MUNITIONS_PRICE_CEILING
        )));
    }

    #[test]
    fn golden_colonize_includes_tech_law_quinine_interest_army_navy_solvent() {
        let goal = parse("colonize(region=region_congo)").unwrap();
        assert!(goal.has_interest_simple_subgoal(), "interest");
        assert!(goal.has_army_simple_subgoal(), "army");
        assert!(goal.has_navy_simple_subgoal(), "navy");
        assert!(goal.has_solvent_simple_subgoal(), "solvent");
        assert!(goal
            .simple_subgoals()
            .iter()
            .any(|a| a.is_has_tech(COLONIZE_TECH)));
        assert!(goal
            .simple_subgoals()
            .iter()
            .any(|a| a.is_has_tech(COLONIZE_QUININE_TECH)));
        assert!(goal
            .simple_subgoals()
            .iter()
            .any(|a| a.is_has_law(LAW_COLONIAL_RESETTLEMENT)
                || a.is_has_law(LAW_COLONIAL_EXPLOITATION)));
        assert!(goal.simple_subgoals().iter().any(|a| matches!(
            a,
            SimpleSubgoal::NavyPower {
                rel: Rel::Ge,
                value,
            } if *value == COLONIZE_NAVY_THRESHOLD
        )));
        assert!(matches!(
            goal.simple_subgoals().iter().find(|a| a.is_interest()),
            Some(SimpleSubgoal::InterestIn {
                kind: InterestKind::Region,
                id,
            }) if id == "region_congo"
        ));
    }

    #[test]
    fn colonize_requires_target() {
        assert_eq!(parse("colonize()").unwrap_err(), GoalError::ColonizeTarget);
    }

    #[test]
    fn golden_research_is_has_tech() {
        let goal = parse("research(tech=nitroglycerin)").unwrap();
        assert_eq!(goal.simple_subgoals().len(), 1);
        assert!(goal.simple_subgoals()[0].is_has_tech("nitroglycerin"));
        assert!(matches!(goal, Goal::Simple(SimpleSubgoal::HasTech(t)) if t == "nitroglycerin"));
    }

    #[test]
    fn golden_good_price_and_solvent() {
        let goal = parse("good_price(ammunition) <= 40 && solvent").unwrap();
        let atoms = goal.simple_subgoals();
        assert_eq!(atoms.len(), 2, "{goal:?}");
        assert!(atoms.iter().any(|a| matches!(
            a,
            SimpleSubgoal::GoodPrice {
                good,
                rel: Rel::Le,
                value,
            } if good == "ammunition" && *value == 40.0
        )));
        assert!(atoms.iter().any(|a| a.is_solvent()));
    }

    #[test]
    fn research_and_numeric_metrics_compile() {
        let tech = parse("has_tech(nitroglycerin)").unwrap();
        assert!(matches!(tech, Goal::Simple(SimpleSubgoal::HasTech(t)) if t == "nitroglycerin"));
        assert!(matches!(
            parse("has_law(law_autocracy)").unwrap(),
            Goal::Simple(SimpleSubgoal::HasLaw(law)) if law == "law_autocracy"
        ));
        let gdp = parse("gdp >= 50e6").unwrap();
        assert!(matches!(
            gdp,
            Goal::Simple(SimpleSubgoal::Gdp {
                rel: Rel::Ge,
                value,
            }) if value == 50e6
        ));
        assert!(matches!(
            parse("weekly_balance >= 100").unwrap(),
            Goal::Simple(SimpleSubgoal::WeeklyBalance {
                rel: Rel::Ge,
                value: 100.0,
            })
        ));
        assert!(matches!(
            parse("population_weighted_wealth >= 20").unwrap(),
            Goal::Simple(SimpleSubgoal::PopulationWeightedWealth {
                rel: Rel::Ge,
                value: 20.0,
            })
        ));
        assert!(matches!(
            parse("credit_headroom > 0").unwrap(),
            Goal::Simple(SimpleSubgoal::CreditHeadroom {
                rel: Rel::Gt,
                value: 0.0,
            })
        ));
        assert!(matches!(
            parse("debt_principal <= 200").unwrap(),
            Goal::Simple(SimpleSubgoal::DebtPrincipal {
                rel: Rel::Le,
                value: 200.0,
            })
        ));
    }

    fn ready_for_alsace() -> PlanningState {
        PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            techs: vec!["nitroglycerin".into()],
            good_prices: vec![("ammunition".into(), 30.0)],
            solvent: true,
            treasury: 5_000.0,
            army_power_projection: Some(150.0),
            interest: vec!["alsace".into()],
            gdp: 60e6,
            weekly_balance: Some(125.0),
            population_weighted_wealth: Some(22.0),
            debt_principal: Some(0.0),
            credit_limit: Some(500.0),
            credit_headroom: Some(500.0),
            ..PlanningParts::default()
        })
    }

    #[test]
    fn eval_fake_state_declare_war_and_research() {
        let state = ready_for_alsace();
        let war = parse("declare-war(tag=FRA, wargoal=conquer_state, state=alsace)").unwrap();
        assert!(evaluate(&war, &state));
        assert!(gaps(&war, &state).is_empty());
        let research = parse("research(tech=nitroglycerin)").unwrap();
        assert!(evaluate(&research, &state));
        assert!(evaluate(&parse("gdp >= 50e6").unwrap(), &state));
        assert!(evaluate(&parse("weekly_balance >= 100").unwrap(), &state));
        assert!(evaluate(
            &parse("population_weighted_wealth >= 20").unwrap(),
            &state
        ));
        assert!(evaluate(&parse("credit_headroom > 0").unwrap(), &state));
        assert!(evaluate(&parse("debt_principal <= 0").unwrap(), &state));
        assert!(evaluate(
            &parse("good_price(ammunition) <= 40 && solvent").unwrap(),
            &state
        ));
    }

    #[test]
    fn gaps_unsatisfied_simple_subgoals_on_fake_state() {
        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            ..PlanningParts::default()
        });
        let war = parse("declare-war(tag=FRA, wargoal=conquer_state, state=alsace)").unwrap();
        assert!(!evaluate(&war, &state));
        let g = gaps(&war, &state);
        assert!(g.iter().any(SimpleSubgoal::is_interest));
        assert!(g.iter().any(SimpleSubgoal::is_army));
        assert!(g.iter().any(SimpleSubgoal::is_munitions_price));
        assert!(g.iter().any(SimpleSubgoal::is_solvent));
        assert_eq!(g.len(), 4);

        let research = parse("research(tech=nitroglycerin)").unwrap();
        let tech_gaps = gaps(&research, &state);
        assert_eq!(tech_gaps.len(), 1);
        assert!(tech_gaps[0].is_has_tech("nitroglycerin"));
        assert!(!evaluate(
            &parse("population_weighted_wealth >= 0").unwrap(),
            &state
        ));
        assert!(!evaluate(&parse("credit_headroom > 0").unwrap(), &state));
    }

    #[test]
    fn gaps_with_defs_include_missing_tech_ancestors() {
        use std::collections::BTreeMap;
        use vic3_defs::{GameDefs, Technology};

        let mut technologies = BTreeMap::new();
        technologies.insert(
            "manufacturies".into(),
            Technology {
                name: "manufacturies".into(),
                cost: Some(50.0),
                prerequisites: vec![],
            },
        );
        technologies.insert(
            "shaft_mining".into(),
            Technology {
                name: "shaft_mining".into(),
                cost: Some(75.0),
                prerequisites: vec!["manufacturies".into()],
            },
        );
        technologies.insert(
            "nitroglycerin".into(),
            Technology {
                name: "nitroglycerin".into(),
                cost: Some(100.0),
                prerequisites: vec!["shaft_mining".into()],
            },
        );
        let defs = GameDefs {
            technologies,
            ..GameDefs::default()
        };
        let state = PlanningState::default();
        let research = parse("research(tech=nitroglycerin)").unwrap();
        let tech_gaps = gaps_with_defs(&research, &state, &defs);
        assert_eq!(tech_gaps.len(), 3);
        assert!(tech_gaps[0].is_has_tech("manufacturies"));
        assert!(tech_gaps[1].is_has_tech("shaft_mining"));
        assert!(tech_gaps[2].is_has_tech("nitroglycerin"));
    }

    #[test]
    fn army_power_unknown_is_not_silent_zero() {
        let unknown = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            ..PlanningParts::default()
        });
        let atom = parse("army_power_projection >= 100").unwrap();
        assert!(matches!(
            atom,
            Goal::Simple(SimpleSubgoal::ArmyPower {
                rel: Rel::Ge,
                value: 100.0
            })
        ));
        let Goal::Simple(army) = &atom else {
            panic!("expected army simple subgoal");
        };
        assert_eq!(army.status(&unknown), SimpleSubgoalStatus::Unknown);
        assert!(!army.eval(&unknown));

        let known_zero = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            army_power_projection: Some(0.0),
            ..PlanningParts::default()
        });
        assert_eq!(army.status(&known_zero), SimpleSubgoalStatus::Failing);

        let ready = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            army_power_projection: Some(150.0),
            ..PlanningParts::default()
        });
        assert_eq!(army.status(&ready), SimpleSubgoalStatus::Cleared);
    }

    #[test]
    fn interest_in_eval_respects_state_vs_region() {
        let state = PlanningState::from_parts(PlanningParts {
            interest: vec!["alsace".into()],
            interest_regions: vec!["region_western_europe".into()],
            ..PlanningParts::default()
        });
        assert!(evaluate(
            &parse("interest_in(state=alsace)").unwrap(),
            &state
        ));
        assert!(!evaluate(
            &parse("interest_in(region=alsace)").unwrap(),
            &state
        ));
        assert!(evaluate(
            &parse("interest_in(region=region_western_europe)").unwrap(),
            &state
        ));
        assert!(!evaluate(
            &parse("interest_in(state=region_western_europe)").unwrap(),
            &state
        ));
    }

    #[test]
    fn not_and_or_eval() {
        let state = ready_for_alsace();
        assert!(!evaluate(&parse("not solvent").unwrap(), &state));
        assert!(evaluate(
            &parse("has_tech(railways) || solvent").unwrap(),
            &state
        ));
        assert!(!evaluate(
            &parse("has_tech(railways) && solvent").unwrap(),
            &state
        ));
    }

    #[test]
    fn parse_rejects_trailing_junk() {
        assert!(parse("solvent oops").is_err());
        assert!(parse("research()").is_err());
        assert!(parse("declare-war(tag=FRA)").is_err());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        /// I-declare-war: compilation always includes interest, army,
        /// munitions-price, and solvent conjuncts.
        #[test]
        fn declare_war_always_includes_interest_army_munitions_price_solvent(
            tag in "[A-Z]{3}",
            wargoal in "[a-z][a-z_]{0,16}",
            state in "[a-z][a-z]{2,12}",
        ) {
            let src = format!(
                "declare-war(tag={tag}, wargoal={wargoal}, state={state})"
            );
            let goal = parse(&src).expect("declare-war parses");
            prop_assert!(goal.has_interest_simple_subgoal(), "interest missing in {src}");
            prop_assert!(goal.has_army_simple_subgoal(), "army missing in {src}");
            prop_assert!(
                goal.has_munitions_price_simple_subgoal(),
                "munitions-price missing in {src}"
            );
            prop_assert!(goal.has_solvent_simple_subgoal(), "solvent missing in {src}");
        }

        /// I-colonize: tech, colonial law, quinine, interest, army, navy, solvent.
        #[test]
        fn colonize_always_includes_readiness_conjuncts(
            region in "[a-z][a-z_]{2,20}",
        ) {
            let src = format!("colonize(region={region})");
            let goal = parse(&src).expect("colonize parses");
            prop_assert!(goal.has_interest_simple_subgoal(), "interest missing in {src}");
            prop_assert!(goal.has_army_simple_subgoal(), "army missing in {src}");
            prop_assert!(goal.has_navy_simple_subgoal(), "navy missing in {src}");
            prop_assert!(goal.has_solvent_simple_subgoal(), "solvent missing in {src}");
            prop_assert!(
                goal.simple_subgoals().iter().any(|a| a.is_has_tech(COLONIZE_TECH)),
                "colonization tech missing in {src}"
            );
            prop_assert!(
                goal.simple_subgoals().iter().any(|a| a.is_has_tech(COLONIZE_QUININE_TECH)),
                "quinine missing in {src}"
            );
        }
    }
}
