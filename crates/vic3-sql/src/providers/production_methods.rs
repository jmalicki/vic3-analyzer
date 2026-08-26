use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{RecordBatch, StringBuilder};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::Session;
use datafusion::datasource::{TableProvider, TableType};
use datafusion::error::Result as DfResult;
use datafusion::logical_expr::{Expr, TableProviderFilterPushDown};
use datafusion::physical_plan::ExecutionPlan;
use vic3_prices::GoodFlow;

use crate::binding::SessionBinding;
use crate::exec::memory_exec;
use crate::schema::production_methods_schema;

use super::arrays::good_io_list_column;
use super::pushdown::{matches_str, PushSupport};

const PUSH: PushSupport = PushSupport {
    eq_u32: &[],
    eq_i32: &[],
    eq_str: &["pm", "pm_name"],
    // defs.production_methods is a BTreeMap — Exact range on pm.
    range_str: &["pm"],
};

#[derive(Debug)]
pub struct ProductionMethodsProvider {
    binding: Arc<SessionBinding>,
    schema: SchemaRef,
}

impl ProductionMethodsProvider {
    pub fn new(binding: Arc<SessionBinding>) -> Self {
        Self {
            binding,
            schema: production_methods_schema(),
        }
    }

    fn batch(&self, filters: &[Expr]) -> DfResult<RecordBatch> {
        let preds = PUSH.collect_preds(filters);
        let mut pm = StringBuilder::new();
        let mut pm_name = StringBuilder::new();
        let mut inputs: Vec<Vec<GoodFlow>> = Vec::new();
        let mut outputs: Vec<Vec<GoodFlow>> = Vec::new();

        for (id, method) in &self.binding.defs.production_methods {
            if !matches_str(&preds, "pm", id) {
                continue;
            }
            let name = self.binding.label(id);
            if let Some(n) = name {
                if !matches_str(&preds, "pm_name", n) {
                    continue;
                }
            } else if preds.iter().any(
                |p| matches!(p, crate::filter::Pred::EqStr { column, .. } if column == "pm_name"),
            ) {
                continue;
            }

            pm.append_value(id);
            match name {
                Some(n) => pm_name.append_value(n),
                None => pm_name.append_null(),
            }
            inputs.push(flows_from_idx(&self.binding, &method.inputs));
            outputs.push(flows_from_idx(&self.binding, &method.outputs));
        }

        RecordBatch::try_new(
            Arc::clone(&self.schema),
            vec![
                Arc::new(pm.finish()),
                Arc::new(pm_name.finish()),
                good_io_list_column(&self.binding, &inputs),
                good_io_list_column(&self.binding, &outputs),
            ],
        )
        .map_err(Into::into)
    }
}

fn flows_from_idx(binding: &SessionBinding, rows: &[(vic3_defs::GoodId, f64)]) -> Vec<GoodFlow> {
    rows.iter()
        .filter_map(|(idx, qty)| {
            let good_id = binding.defs.good_by_index(*idx)?.to_string();
            Some(GoodFlow {
                good_name: good_id,
                quantity: *qty,
                value: 0.0,
            })
        })
        .collect()
}

#[async_trait]
impl TableProvider for ProductionMethodsProvider {
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
