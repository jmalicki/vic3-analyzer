//! Chumsky combinators for [`docs/dsl.md`](../../../docs/dsl.md). Keep this small.

use chumsky::prelude::*;

use crate::{
    Atom, Goal, GoalError, InterestKind, Rel, DECLARE_WAR_ARMY_THRESHOLD,
    DECLARE_WAR_MUNITIONS_PRICE_CEILING, MUNITIONS_GOOD,
};

type Err<'src> = extra::Err<Rich<'src, char>>;

#[derive(Debug, Clone)]
pub(crate) enum Expr {
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Not(Box<Expr>),
    Pred(RawPred),
}

#[derive(Debug, Clone)]
pub(crate) struct RawPred {
    name: String,
    args: Vec<Arg>,
    rel: Option<(Rel, f64)>,
}

#[derive(Debug, Clone)]
enum Arg {
    Named(String, Value),
    Positional(Value),
}

#[derive(Debug, Clone)]
enum Value {
    Ident(String),
    /// Numeric arg (`foo=1.5`); sugar does not read these yet.
    Number(f64),
    Str(String),
}

impl Value {
    fn as_ident(&self) -> Option<String> {
        match self {
            Value::Ident(s) | Value::Str(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
        }
    }
}

/// Parse a goal string and compile sugar. Parser entrypoint (also exported as
/// [`crate::parse`]).
pub fn parse(src: &str) -> Result<Goal, GoalError> {
    let expr = parse_expr(src)?;
    compile_expr(expr)
}

fn parse_expr(src: &str) -> Result<Expr, GoalError> {
    parser().parse(src.trim()).into_result().map_err(|errs| {
        GoalError::Parse(
            errs.into_iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; "),
        )
    })
}

fn parser<'src>() -> impl Parser<'src, &'src str, Expr, Err<'src>> {
    recursive(|goal| {
        let ident = ident_parser().padded();
        let number = number_parser().padded();
        let string = string_parser().padded();

        let value = choice((
            number.clone().map(Value::Number),
            string,
            ident.clone().map(Value::Ident),
        ));

        let named = ident
            .clone()
            .then_ignore(just('=').padded())
            .then(value.clone())
            .map(|(k, v)| Arg::Named(k, v));
        let arg = named.or(value.map(Arg::Positional));

        let args = arg
            .separated_by(just(',').padded())
            .collect::<Vec<_>>()
            .delimited_by(just('(').padded(), just(')').padded());

        let rel = choice((
            just("<=").to(Rel::Le),
            just(">=").to(Rel::Ge),
            just("==").to(Rel::Eq),
            just('<').to(Rel::Lt),
            just('>').to(Rel::Gt),
        ))
        .padded();

        let pred = ident
            .then(args.or_not())
            .then(rel.then(number).or_not())
            .map(|((name, args), rel)| {
                Expr::Pred(RawPred {
                    name,
                    args: args.unwrap_or_default(),
                    rel,
                })
            });

        let atom = goal
            .delimited_by(just('(').padded(), just(')').padded())
            .or(pred);

        let unary = recursive(|unary| {
            text::ascii::keyword("not")
                .padded()
                .ignore_then(unary)
                .map(|e| Expr::Not(Box::new(e)))
                .or(atom)
        });

        let and_expr = unary
            .separated_by(just("&&").padded())
            .at_least(1)
            .collect::<Vec<_>>()
            .map(|xs| {
                if xs.len() == 1 {
                    xs.into_iter().next().unwrap()
                } else {
                    Expr::And(xs)
                }
            });

        and_expr
            .separated_by(just("||").padded())
            .at_least(1)
            .collect::<Vec<_>>()
            .map(|xs| {
                if xs.len() == 1 {
                    xs.into_iter().next().unwrap()
                } else {
                    Expr::Or(xs)
                }
            })
    })
    .then_ignore(end())
}

fn ident_parser<'src>() -> impl Parser<'src, &'src str, String, Err<'src>> + Clone {
    text::ascii::ident()
        .then(just('-').then(text::ascii::ident()).repeated())
        .to_slice()
        .map(str::to_string)
}

fn number_parser<'src>() -> impl Parser<'src, &'src str, f64, Err<'src>> + Clone {
    let digits = text::digits(10).to_slice();
    just('-')
        .or_not()
        .then(text::int(10))
        .then(just('.').then(digits).or_not())
        .then(
            one_of("eE")
                .then(one_of("+-").or_not())
                .then(text::int(10))
                .or_not(),
        )
        .to_slice()
        .try_map(|s: &str, span| {
            s.parse::<f64>()
                .map_err(|_| Rich::custom(span, format!("invalid number `{s}`")))
        })
}

fn string_parser<'src>() -> impl Parser<'src, &'src str, Value, Err<'src>> + Clone {
    none_of('"')
        .repeated()
        .to_slice()
        .delimited_by(just('"'), just('"'))
        .map(|s: &str| Value::Str(s.to_string()))
}

fn compile_expr(expr: Expr) -> Result<Goal, GoalError> {
    match expr {
        Expr::And(xs) => Ok(Goal::from_and(
            xs.into_iter()
                .map(compile_expr)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Expr::Or(xs) => Ok(Goal::from_or(
            xs.into_iter()
                .map(compile_expr)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Expr::Not(inner) => Ok(Goal::Not(Box::new(compile_expr(*inner)?))),
        Expr::Pred(pred) => compile_pred(pred),
    }
}

fn compile_pred(pred: RawPred) -> Result<Goal, GoalError> {
    match pred.name.as_str() {
        "declare-war" => compile_declare_war(&pred),
        "research" => compile_research(&pred),
        "has_tech" => {
            let tech = first_ident(&pred, "tech").ok_or(GoalError::ResearchTech)?;
            Ok(Goal::Atom(Atom::HasTech(tech)))
        }
        "solvent" if pred.args.is_empty() && pred.rel.is_none() => Ok(Goal::Atom(Atom::Solvent)),
        "interest_in" => {
            if let Some(id) = named(&pred, "state") {
                Ok(Goal::Atom(Atom::InterestIn {
                    kind: InterestKind::State,
                    id,
                }))
            } else if let Some(id) = named(&pred, "region") {
                Ok(Goal::Atom(Atom::InterestIn {
                    kind: InterestKind::Region,
                    id,
                }))
            } else if let Some(id) = first_ident(&pred, "state") {
                Ok(Goal::Atom(Atom::InterestIn {
                    kind: InterestKind::State,
                    id,
                }))
            } else {
                Err(GoalError::DeclareWarTarget)
            }
        }
        "good_price" => {
            let good = first_ident(&pred, "good")
                .ok_or_else(|| GoalError::Parse("good_price requires a good id".into()))?;
            let (rel, value) = pred
                .rel
                .ok_or_else(|| GoalError::Parse("good_price requires a comparison".into()))?;
            Ok(Goal::Atom(Atom::GoodPrice { good, rel, value }))
        }
        "army_power_projection" => {
            let (rel, value) = pred.rel.ok_or_else(|| {
                GoalError::Parse("army_power_projection requires a comparison".into())
            })?;
            Ok(Goal::Atom(Atom::ArmyPower { rel, value }))
        }
        "gdp" => {
            let (rel, value) = pred
                .rel
                .ok_or_else(|| GoalError::Parse("gdp requires a comparison".into()))?;
            Ok(Goal::Atom(Atom::Gdp { rel, value }))
        }
        other => Err(GoalError::Parse(format!("unknown predicate `{other}`"))),
    }
}

/// Expand `declare-war(...)` to interest ∧ army ∧ munitions-price ∧ solvent.
fn compile_declare_war(pred: &RawPred) -> Result<Goal, GoalError> {
    let (kind, id) = if let Some(id) = named(pred, "state") {
        (InterestKind::State, id)
    } else if let Some(id) = named(pred, "region") {
        (InterestKind::Region, id)
    } else {
        return Err(GoalError::DeclareWarTarget);
    };
    Ok(Goal::And(vec![
        Goal::Atom(Atom::InterestIn { kind, id }),
        Goal::Atom(Atom::ArmyPower {
            rel: Rel::Ge,
            value: DECLARE_WAR_ARMY_THRESHOLD,
        }),
        Goal::Atom(Atom::GoodPrice {
            good: MUNITIONS_GOOD.into(),
            rel: Rel::Le,
            value: DECLARE_WAR_MUNITIONS_PRICE_CEILING,
        }),
        Goal::Atom(Atom::Solvent),
    ]))
}

fn compile_research(pred: &RawPred) -> Result<Goal, GoalError> {
    let tech = first_ident(pred, "tech").ok_or(GoalError::ResearchTech)?;
    Ok(Goal::Atom(Atom::HasTech(tech)))
}

fn named(pred: &RawPred, key: &str) -> Option<String> {
    pred.args.iter().find_map(|arg| match arg {
        Arg::Named(k, v) if k == key => v.as_ident(),
        _ => None,
    })
}

fn first_ident(pred: &RawPred, key: &str) -> Option<String> {
    if let Some(v) = named(pred, key) {
        return Some(v);
    }
    pred.args.iter().find_map(|arg| match arg {
        Arg::Positional(v) => v.as_ident(),
        Arg::Named(_, _) => None,
    })
}
