//! `plan(goal [, max_days [, label]])` → ordered A* step rows.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{Int32Builder, RecordBatch, StringBuilder, UInt32Builder};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::{Session, TableFunctionImpl};
use datafusion::common::{plan_err, Result as DfResult};
use datafusion::datasource::{TableProvider, TableType};
use datafusion::logical_expr::Expr;
use datafusion::physical_plan::ExecutionPlan;
use serde_json::Value;
use vic3_plan::plan_with_economy;
use vic3_prices::SolveOpts;
use vic3_sim::{Action, EconomyContext, SimConfig};
use vic3_world::PlanningState;

use crate::binding::SessionBinding;
use crate::exec::memory_exec;
use crate::schema::plan_schema;
use crate::udfs::{literal_str, literal_u32};

#[derive(Debug)]
pub struct PlanTvf {
    binding: Arc<SessionBinding>,
}

impl PlanTvf {
    pub fn new(binding: Arc<SessionBinding>) -> Self {
        Self { binding }
    }
}

impl TableFunctionImpl for PlanTvf {
    fn call(&self, args: &[Expr]) -> DfResult<Arc<dyn TableProvider>> {
        if args.is_empty() || args.len() > 3 {
            return plan_err!("plan(goal [, max_days [, label]]) expects 1 to 3 arguments");
        }
        let goal_src = literal_str(&args[0], 1)?;
        let max_days = if args.len() >= 2 {
            literal_u32(&args[1], 2)?
        } else {
            // Mirror [`PlanOpts`] serde default (`docs/json-schema.md`).
            3650
        };
        // Optional label mirrors PlanOpts; plan rows do not emit it.
        if args.len() == 3 {
            let _label = literal_str(&args[2], 3)?;
        }

        let goal = vic3_goals::compile(&goal_src)
            .map_err(|e| datafusion::common::DataFusionError::Plan(format!("plan goal: {e}")))?;
        let country = self.binding.world.player_country_tag().ok_or_else(|| {
            datafusion::common::DataFusionError::Execution(
                "plan: save has no playable country".into(),
            )
        })?;
        let state = PlanningState::from_world_with_prices(
            &self.binding.world,
            country,
            &self.binding.prices,
        )
        .map_err(|e| datafusion::common::DataFusionError::Execution(format!("plan state: {e}")))?;
        let economy = EconomyContext::new(
            (*self.binding.world).clone(),
            (*self.binding.defs).clone(),
            SolveOpts::default(),
        );
        let result = plan_with_economy(
            state,
            goal,
            SimConfig::default(),
            economy,
            max_days,
            self.binding.prices.residual,
            self.binding.prices.limitations.clone(),
        )
        .map_err(|e| datafusion::common::DataFusionError::Execution(format!("plan: {e}")))?;

        let batch = plan_batch(&result.actions, &result.limitations)?;
        Ok(Arc::new(PlanProvider {
            schema: plan_schema(),
            batch,
        }))
    }
}

fn plan_batch(actions: &[vic3_plan::PlanStep], limitations: &[String]) -> DfResult<RecordBatch> {
    let schema = plan_schema();
    let mut step = Int32Builder::with_capacity(actions.len());
    let mut day = UInt32Builder::with_capacity(actions.len());
    let mut action = StringBuilder::with_capacity(actions.len(), actions.len() * 16);
    let mut detail = StringBuilder::with_capacity(actions.len(), actions.len() * 32);
    let mut lim = StringBuilder::with_capacity(actions.len(), 64);

    let joined_lim = if limitations.is_empty() {
        None
    } else {
        Some(limitations.join("; "))
    };

    for (i, plan_step) in actions.iter().enumerate() {
        step.append_value(i as i32);
        day.append_value(plan_step.day);
        let (verb, args_json) = action_columns(&plan_step.action);
        action.append_value(&verb);
        detail.append_value(&args_json);
        if i == 0 {
            match &joined_lim {
                Some(s) => lim.append_value(s),
                None => lim.append_null(),
            }
        } else {
            lim.append_null();
        }
    }

    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(step.finish()),
            Arc::new(day.finish()),
            Arc::new(action.finish()),
            Arc::new(detail.finish()),
            Arc::new(lim.finish()),
        ],
    )
    .map_err(Into::into)
}

fn action_columns(action: &Action) -> (String, String) {
    let value = serde_json::to_value(action).unwrap_or(Value::Null);
    match value {
        Value::Object(map) => {
            if let Some((key, payload)) = map.into_iter().next() {
                (key, payload.to_string())
            } else {
                ("unknown".into(), "{}".into())
            }
        }
        other => ("unknown".into(), other.to_string()),
    }
}

#[derive(Debug)]
struct PlanProvider {
    schema: SchemaRef,
    batch: RecordBatch,
}

#[async_trait]
impl TableProvider for PlanProvider {
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
