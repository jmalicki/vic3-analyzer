//! `gaps(goal)` → one row per goal atom with failing/cleared status.
//!
//! `goal` must be a non-null string literal. `status` is `cleared` \| `failing`;
//! `detail` is the atom JSON.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{RecordBatch, StringBuilder};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::{Session, TableFunctionImpl};
use datafusion::common::{plan_err, Result as DfResult};
use datafusion::datasource::{TableProvider, TableType};
use datafusion::logical_expr::Expr;
use datafusion::physical_plan::ExecutionPlan;
use vic3_goals::{Atom, InterestKind, Rel};
use vic3_world::PlanningState;

use crate::binding::SessionBinding;
use crate::exec::memory_exec;
use crate::schema::gaps_schema;
use crate::udfs::literal_str;

#[derive(Debug)]
pub struct GapsTvf {
    binding: Arc<SessionBinding>,
}

impl GapsTvf {
    pub fn new(binding: Arc<SessionBinding>) -> Self {
        Self { binding }
    }
}

impl TableFunctionImpl for GapsTvf {
    fn call(&self, args: &[Expr]) -> DfResult<Arc<dyn TableProvider>> {
        if args.len() != 1 {
            return plan_err!("gaps(goal) expects exactly 1 argument");
        }
        let goal_src = literal_str(&args[0], 1)?;
        let goal = vic3_goals::compile(&goal_src)
            .map_err(|e| datafusion::common::DataFusionError::Plan(format!("gaps goal: {e}")))?;
        let country = self.binding.world.player_country_tag().ok_or_else(|| {
            datafusion::common::DataFusionError::Execution(
                "gaps: save has no playable country".into(),
            )
        })?;
        let state = PlanningState::from_world_with_prices(
            &self.binding.world,
            country,
            &self.binding.prices,
        )
        .map_err(|e| datafusion::common::DataFusionError::Execution(format!("gaps state: {e}")))?;

        let batch = gaps_batch(&goal, &state)?;
        Ok(Arc::new(GapsProvider {
            schema: gaps_schema(),
            batch,
        }))
    }
}

fn gaps_batch(goal: &vic3_goals::Goal, state: &PlanningState) -> DfResult<RecordBatch> {
    let atoms = goal.atoms();
    let schema = gaps_schema();
    let mut predicate = StringBuilder::with_capacity(atoms.len(), atoms.len() * 24);
    let mut status = StringBuilder::with_capacity(atoms.len(), atoms.len() * 8);
    let mut detail = StringBuilder::with_capacity(atoms.len(), atoms.len() * 32);

    for atom in atoms {
        predicate.append_value(format_atom(atom));
        if atom.eval(state) {
            status.append_value("cleared");
        } else {
            status.append_value("failing");
        }
        detail.append_value(serde_json::to_string(atom).unwrap_or_else(|_| "{}".into()));
    }

    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(predicate.finish()),
            Arc::new(status.finish()),
            Arc::new(detail.finish()),
        ],
    )
    .map_err(Into::into)
}

fn format_atom(atom: &Atom) -> String {
    match atom {
        Atom::HasTech(tech) => format!("has_tech({tech})"),
        Atom::HasLaw(law) => format!("has_law({law})"),
        Atom::GoodPrice { good, rel, value } => {
            format!("good_price({good}) {} {value}", rel_str(*rel))
        }
        Atom::ArmyPower { rel, value } => {
            format!("army_power_projection {} {value}", rel_str(*rel))
        }
        Atom::Solvent => "solvent".into(),
        Atom::InterestIn {
            kind: InterestKind::State,
            id,
        } => format!("interest_in(state={id})"),
        Atom::InterestIn {
            kind: InterestKind::Region,
            id,
        } => format!("interest_in(region={id})"),
        Atom::Gdp { rel, value } => format!("gdp {} {value}", rel_str(*rel)),
        Atom::WeeklyBalance { rel, value } => {
            format!("weekly_balance {} {value}", rel_str(*rel))
        }
        Atom::PopulationWeightedWealth { rel, value } => {
            format!("population_weighted_wealth {} {value}", rel_str(*rel))
        }
        Atom::DebtPrincipal { rel, value } => {
            format!("debt_principal {} {value}", rel_str(*rel))
        }
        Atom::CreditHeadroom { rel, value } => {
            format!("credit_headroom {} {value}", rel_str(*rel))
        }
    }
}

fn rel_str(rel: Rel) -> &'static str {
    rel.as_str()
}

#[derive(Debug)]
struct GapsProvider {
    schema: SchemaRef,
    batch: RecordBatch,
}

#[async_trait]
impl TableProvider for GapsProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn table_type(&self) -> TableType {
        TableType::Temporary
    }

    async fn scan(
        &self,
        _state: &dyn Session,
        projection: Option<&Vec<usize>>,
        _filters: &[Expr],
        _limit: Option<usize>,
    ) -> DfResult<Arc<dyn ExecutionPlan>> {
        memory_exec(self.batch.clone(), projection)
    }
}
