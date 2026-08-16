//! Goal DSL: chumsky parser, sugar compilation, evaluation against
//! [`vic3_world::PlanningState`]. This crate does not search.

mod parse;

use serde::{Deserialize, Serialize};
use vic3_world::PlanningState;

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

/// Good id used as the munitions-price atom when compiling `declare-war`.
pub const MUNITIONS_GOOD: &str = "ammunition";

/// Parse / compile failure.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum GoalError {
    #[error("failed to parse goal: {0}")]
    Parse(String),
    #[error("declare-war requires state= or region=")]
    DeclareWarTarget,
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

/// Where [`Atom::InterestIn`] points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InterestKind {
    State,
    Region,
}

/// A compiled leaf predicate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Atom {
    HasTech(String),
    GoodPrice { good: String, rel: Rel, value: f64 },
    ArmyPower { rel: Rel, value: f64 },
    Solvent,
    InterestIn { kind: InterestKind, id: String },
    Gdp { rel: Rel, value: f64 },
    WeeklyBalance { rel: Rel, value: f64 },
    PopulationWeightedWealth { rel: Rel, value: f64 },
    DebtPrincipal { rel: Rel, value: f64 },
    CreditHeadroom { rel: Rel, value: f64 },
}

impl Atom {
    pub fn is_interest(&self) -> bool {
        matches!(self, Atom::InterestIn { .. })
    }

    pub fn is_army(&self) -> bool {
        matches!(self, Atom::ArmyPower { .. })
    }

    pub fn is_munitions_price(&self) -> bool {
        matches!(self, Atom::GoodPrice { good, .. } if good == MUNITIONS_GOOD)
    }

    pub fn is_solvent(&self) -> bool {
        matches!(self, Atom::Solvent)
    }

    pub fn is_has_tech(&self, tech: &str) -> bool {
        matches!(self, Atom::HasTech(t) if t == tech)
    }

    pub fn eval(&self, state: &PlanningState) -> bool {
        match self {
            Atom::HasTech(tech) => state.has_tech(tech),
            Atom::GoodPrice { good, rel, value } => state
                .price(good)
                .map(|p| rel.holds(p, *value))
                .unwrap_or(false),
            Atom::ArmyPower { rel, value } => rel.holds(state.army_power_projection, *value),
            Atom::Solvent => state.solvent,
            Atom::InterestIn { id, .. } => state.has_interest(id),
            Atom::Gdp { rel, value } => rel.holds(state.gdp, *value),
            Atom::WeeklyBalance { rel, value } => state
                .weekly_balance
                .map(|balance| rel.holds(balance, *value))
                .unwrap_or(false),
            Atom::PopulationWeightedWealth { rel, value } => state
                .population_weighted_wealth
                .map(|wealth| rel.holds(wealth, *value))
                .unwrap_or(false),
            Atom::DebtPrincipal { rel, value } => state
                .debt_principal
                .map(|principal| rel.holds(principal, *value))
                .unwrap_or(false),
            Atom::CreditHeadroom { rel, value } => state
                .credit_headroom
                .map(|headroom| rel.holds(headroom, *value))
                .unwrap_or(false),
        }
    }
}

/// Compiled boolean formula over a [`PlanningState`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Goal {
    And(Vec<Goal>),
    Or(Vec<Goal>),
    Not(Box<Goal>),
    Atom(Atom),
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

    /// Flattened atoms (including under `not`).
    pub fn atoms(&self) -> Vec<&Atom> {
        let mut out = Vec::new();
        self.collect_atoms(&mut out);
        out
    }

    fn collect_atoms<'a>(&'a self, out: &mut Vec<&'a Atom>) {
        match self {
            Goal::And(xs) | Goal::Or(xs) => {
                for x in xs {
                    x.collect_atoms(out);
                }
            }
            Goal::Not(inner) => inner.collect_atoms(out),
            Goal::Atom(atom) => out.push(atom),
        }
    }

    pub fn has_interest_atom(&self) -> bool {
        self.atoms().iter().any(|a| a.is_interest())
    }

    pub fn has_army_atom(&self) -> bool {
        self.atoms().iter().any(|a| a.is_army())
    }

    pub fn has_munitions_price_atom(&self) -> bool {
        self.atoms().iter().any(|a| a.is_munitions_price())
    }

    pub fn has_solvent_atom(&self) -> bool {
        self.atoms().iter().any(|a| a.is_solvent())
    }
}

/// Parse the DSL and compile sugar (`declare-war`, `research`, `gdp`).
///
/// This is the parser entrypoint.
pub fn compile(src: &str) -> Result<Goal, GoalError> {
    parse(src)
}

/// Evaluate a compiled goal against a planning projection. Does not search.
pub fn evaluate(goal: &Goal, state: &PlanningState) -> bool {
    match goal {
        Goal::And(xs) => xs.iter().all(|g| evaluate(g, state)),
        Goal::Or(xs) => xs.iter().any(|g| evaluate(g, state)),
        Goal::Not(inner) => !evaluate(inner, state),
        Goal::Atom(atom) => atom.eval(state),
    }
}

/// Unsatisfied atoms (empty when [`evaluate`] is true). Does not search.
pub fn gaps(goal: &Goal, state: &PlanningState) -> Vec<Atom> {
    let mut out = Vec::new();
    collect_gaps(goal, state, &mut out);
    out
}

fn collect_gaps(goal: &Goal, state: &PlanningState, out: &mut Vec<Atom>) {
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
            flatten_atoms(inner, out);
        }
        Goal::Atom(atom) => out.push(atom.clone()),
    }
}

fn flatten_atoms(goal: &Goal, out: &mut Vec<Atom>) {
    match goal {
        Goal::And(xs) | Goal::Or(xs) => {
            for x in xs {
                flatten_atoms(x, out);
            }
        }
        Goal::Not(inner) => flatten_atoms(inner, out),
        Goal::Atom(atom) => out.push(atom.clone()),
    }
}

/// Crate version from Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use vic3_world::{PlanningParts, PlanningState};

    #[test]
    fn version_is_semver() {
        assert!(!version().is_empty());
    }

    #[test]
    fn golden_declare_war_includes_interest_army_munitions_price_solvent() {
        let goal = parse("declare-war(tag=FRA, wargoal=conquer_state, state=alsace)").unwrap();
        assert!(goal.has_interest_atom(), "interest");
        assert!(goal.has_army_atom(), "army");
        assert!(goal.has_munitions_price_atom(), "munitions-price");
        assert!(goal.has_solvent_atom(), "solvent");
        let interest = goal
            .atoms()
            .into_iter()
            .find(|a| a.is_interest())
            .expect("interest atom");
        assert_eq!(
            interest,
            &Atom::InterestIn {
                kind: InterestKind::State,
                id: "alsace".into(),
            }
        );
        assert!(goal.atoms().iter().any(|a| matches!(
            a,
            Atom::ArmyPower {
                rel: Rel::Ge,
                value,
            } if *value == DECLARE_WAR_ARMY_THRESHOLD
        )));
        assert!(goal.atoms().iter().any(|a| matches!(
            a,
            Atom::GoodPrice {
                good,
                rel: Rel::Le,
                value,
            } if good == MUNITIONS_GOOD && *value == DECLARE_WAR_MUNITIONS_PRICE_CEILING
        )));
    }

    #[test]
    fn golden_research_is_has_tech() {
        let goal = parse("research(tech=nitroglycerin)").unwrap();
        assert_eq!(goal.atoms().len(), 1);
        assert!(goal.atoms()[0].is_has_tech("nitroglycerin"));
        assert!(matches!(goal, Goal::Atom(Atom::HasTech(t)) if t == "nitroglycerin"));
    }

    #[test]
    fn golden_good_price_and_solvent() {
        let goal = parse("good_price(ammunition) <= 40 && solvent").unwrap();
        let atoms = goal.atoms();
        assert_eq!(atoms.len(), 2, "{goal:?}");
        assert!(atoms.iter().any(|a| matches!(
            a,
            Atom::GoodPrice {
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
        assert!(matches!(tech, Goal::Atom(Atom::HasTech(t)) if t == "nitroglycerin"));
        let gdp = parse("gdp >= 50e6").unwrap();
        assert!(matches!(
            gdp,
            Goal::Atom(Atom::Gdp {
                rel: Rel::Ge,
                value,
            }) if value == 50e6
        ));
        assert!(matches!(
            parse("weekly_balance >= 100").unwrap(),
            Goal::Atom(Atom::WeeklyBalance {
                rel: Rel::Ge,
                value: 100.0,
            })
        ));
        assert!(matches!(
            parse("population_weighted_wealth >= 20").unwrap(),
            Goal::Atom(Atom::PopulationWeightedWealth {
                rel: Rel::Ge,
                value: 20.0,
            })
        ));
        assert!(matches!(
            parse("credit_headroom > 0").unwrap(),
            Goal::Atom(Atom::CreditHeadroom {
                rel: Rel::Gt,
                value: 0.0,
            })
        ));
        assert!(matches!(
            parse("debt_principal <= 200").unwrap(),
            Goal::Atom(Atom::DebtPrincipal {
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
            army_power_projection: 150.0,
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
    fn gaps_unsatisfied_atoms_on_fake_state() {
        let state = PlanningState::from_parts(PlanningParts {
            country: "GER".into(),
            ..PlanningParts::default()
        });
        let war = parse("declare-war(tag=FRA, wargoal=conquer_state, state=alsace)").unwrap();
        assert!(!evaluate(&war, &state));
        let g = gaps(&war, &state);
        assert!(g.iter().any(Atom::is_interest));
        assert!(g.iter().any(Atom::is_army));
        assert!(g.iter().any(Atom::is_munitions_price));
        assert!(g.iter().any(Atom::is_solvent));
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
            prop_assert!(goal.has_interest_atom(), "interest missing in {src}");
            prop_assert!(goal.has_army_atom(), "army missing in {src}");
            prop_assert!(
                goal.has_munitions_price_atom(),
                "munitions-price missing in {src}"
            );
            prop_assert!(goal.has_solvent_atom(), "solvent missing in {src}");
        }
    }
}
