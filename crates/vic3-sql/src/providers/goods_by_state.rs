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

use crate::binding::{goods_shortage, SessionBinding};
use crate::exec::memory_exec;
use crate::schema::goods_by_state_schema;

use super::pushdown::{matches_str, matches_u32, PushSupport};

const PUSH: PushSupport = PushSupport {
    eq_u32: &["state_id"],
    eq_i32: &[],
    eq_str: &["good"],
    range_str: &[],
};

#[derive(Debug)]
pub struct GoodsByStateProvider {
    binding: Arc<SessionBinding>,
    schema: SchemaRef,
}

impl GoodsByStateProvider {
    pub fn new(binding: Arc<SessionBinding>) -> Self {
        Self {
            binding,
            schema: goods_by_state_schema(),
        }
    }

    fn batch(&self, filters: &[Expr]) -> DfResult<RecordBatch> {
        let preds = PUSH.collect_preds(filters);
        let mut state_id = UInt32Builder::new();
        let mut good = StringBuilder::new();
        let mut price = Float64Builder::new();
        let mut buy = Float64Builder::new();
        let mut sell = Float64Builder::new();
        let mut shortage = Float64Builder::new();

        let mut rows = self.binding.prices.state_goods.clone();
        rows.sort_by(|a, b| a.state_id.cmp(&b.state_id).then(a.good_id.cmp(&b.good_id)));

        for g in &rows {
            if !matches_u32(&preds, "state_id", g.state_id) {
                continue;
            }
            if !matches_str(&preds, "good", &g.good_id) {
                continue;
            }
            state_id.append_value(g.state_id);
            good.append_value(&g.good_id);
            price.append_value(g.price);
            buy.append_value(g.buy);
            sell.append_value(g.sell);
            shortage.append_value(goods_shortage(g.buy, g.sell));
        }

        RecordBatch::try_new(
            Arc::clone(&self.schema),
            vec![
                Arc::new(state_id.finish()),
                Arc::new(good.finish()),
                Arc::new(price.finish()),
                Arc::new(buy.finish()),
                Arc::new(sell.finish()),
                Arc::new(shortage.finish()),
            ],
        )
        .map_err(Into::into)
    }
}

#[async_trait]
impl TableProvider for GoodsByStateProvider {
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
