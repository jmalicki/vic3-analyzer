use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{Float64Builder, RecordBatch, StringBuilder, UInt32Builder};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::Session;
use datafusion::datasource::{TableProvider, TableType};
use datafusion::error::Result as DfResult;
use datafusion::logical_expr::{Expr, TableProviderFilterPushDown};
use datafusion::physical_plan::ExecutionPlan;

use crate::binding::SessionBinding;
use crate::exec::memory_exec;
use crate::schema::pops_schema;

use super::pushdown::{matches_str, matches_u32, PushSupport};

const PUSH: PushSupport = PushSupport {
    eq_u32: &["state_id"],
    eq_i32: &[],
    eq_str: &["profession"],
    range_str: &[],
};

#[derive(Debug)]
pub struct PopsProvider {
    binding: Arc<SessionBinding>,
    schema: SchemaRef,
}

impl PopsProvider {
    pub fn new(binding: Arc<SessionBinding>) -> Self {
        Self {
            binding,
            schema: pops_schema(),
        }
    }

    fn batch(&self, filters: &[Expr]) -> DfResult<RecordBatch> {
        let preds = PUSH.collect_preds(filters);
        let mut state_id = UInt32Builder::new();
        let mut profession = StringBuilder::new();
        let mut workforce = Float64Builder::new();
        let mut dependents = Float64Builder::new();
        let mut literacy = Float64Builder::new();

        for pop in self.binding.prices.state_pops.iter() {
            if !matches_u32(&preds, "state_id", pop.state_id) {
                continue;
            }
            if let Some(p) = &pop.profession_id {
                if !matches_str(&preds, "profession", p) {
                    continue;
                }
            } else if preds.iter().any(|p| {
                matches!(p, crate::filter::Pred::EqStr { column, .. } if column == "profession")
            }) {
                continue;
            }

            state_id.append_value(pop.state_id);
            match &pop.profession_id {
                Some(p) => profession.append_value(p),
                None => profession.append_null(),
            }
            match pop.workforce {
                Some(v) => workforce.append_value(v),
                None => workforce.append_null(),
            }
            match pop.dependents {
                Some(v) => dependents.append_value(v),
                None => dependents.append_null(),
            }
            match pop.literate {
                Some(v) => literacy.append_value(v),
                None => literacy.append_null(),
            }
        }

        RecordBatch::try_new(
            Arc::clone(&self.schema),
            vec![
                Arc::new(state_id.finish()),
                Arc::new(profession.finish()),
                Arc::new(workforce.finish()),
                Arc::new(dependents.finish()),
                Arc::new(literacy.finish()),
            ],
        )
        .map_err(Into::into)
    }
}

#[async_trait]
impl TableProvider for PopsProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DfResult<Vec<TableProviderFilterPushDown>> {
        Ok(PUSH.classify(filters))
    }

    async fn scan(
        &self,
        _state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        _limit: Option<usize>,
    ) -> DfResult<Arc<dyn ExecutionPlan>> {
        memory_exec(self.batch(filters)?, projection)
    }
}
