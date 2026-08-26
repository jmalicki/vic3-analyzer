use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{Float64Builder, RecordBatch, StringBuilder};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::Session;
use datafusion::datasource::{TableProvider, TableType};
use datafusion::error::Result as DfResult;
use datafusion::logical_expr::{Expr, TableProviderFilterPushDown};
use datafusion::physical_plan::ExecutionPlan;

use crate::binding::{goods_shortage, SessionBinding};
use crate::exec::memory_exec;
use crate::filter::Pred;
use crate::schema::goods_schema;

use super::pushdown::{matches_str, PushSupport};

const PUSH: PushSupport = PushSupport {
    eq_u32: &[],
    eq_i32: &[],
    // goods_order / index_of / label hash — Exact equality only (no range).
    eq_str: &["good_name", "good_label"],
    range_str: &[],
};

#[derive(Debug)]
pub struct GoodsProvider {
    binding: Arc<SessionBinding>,
    schema: SchemaRef,
    by_good: HashMap<String, usize>,
    by_label: HashMap<String, Vec<usize>>,
}

impl GoodsProvider {
    pub fn new(binding: Arc<SessionBinding>) -> Self {
        let mut by_good = HashMap::new();
        let mut by_label: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, g) in binding.prices.goods.iter().enumerate() {
            by_good.insert(g.good_name.clone(), i);
            if let Some(n) = &g.good_label {
                by_label.entry(n.clone()).or_default().push(i);
            }
        }
        Self {
            binding,
            schema: goods_schema(),
            by_good,
            by_label,
        }
    }

    fn batch(&self, filters: &[Expr]) -> DfResult<RecordBatch> {
        let preds = PUSH.collect_preds(filters);
        let indices: Vec<usize> = if let Some(Pred::EqStr { column, value }) = preds
            .iter()
            .find(|p| matches!(p, Pred::EqStr { column, .. } if column == "good_name"))
        {
            let _ = column;
            self.by_good.get(value).copied().into_iter().collect()
        } else if let Some(Pred::EqStr { column, value }) = preds
            .iter()
            .find(|p| matches!(p, Pred::EqStr { column, .. } if column == "good_label"))
        {
            let _ = column;
            self.by_label.get(value).cloned().unwrap_or_default()
        } else {
            (0..self.binding.prices.goods.len()).collect()
        };

        let mut good_name = StringBuilder::new();
        let mut good_label = StringBuilder::new();
        let mut base = Float64Builder::new();
        let mut price = Float64Builder::new();
        let mut buy = Float64Builder::new();
        let mut sell = Float64Builder::new();
        let mut shortage = Float64Builder::new();

        for i in indices {
            let g = &self.binding.prices.goods[i];
            if !matches_str(&preds, "good_name", &g.good_name) {
                continue;
            }
            if let Some(n) = &g.good_label {
                if !matches_str(&preds, "good_label", n) {
                    continue;
                }
            } else if preds
                .iter()
                .any(|p| matches!(p, Pred::EqStr { column, .. } if column == "good_label"))
            {
                continue;
            }
            good_name.append_value(&g.good_name);
            match &g.good_label {
                Some(n) => good_label.append_value(n),
                None => good_label.append_null(),
            }
            base.append_value(g.base);
            price.append_value(g.price);
            buy.append_value(g.buy);
            sell.append_value(g.sell);
            shortage.append_value(goods_shortage(g.buy, g.sell));
        }

        RecordBatch::try_new(
            Arc::clone(&self.schema),
            vec![
                Arc::new(good_name.finish()),
                Arc::new(good_label.finish()),
                Arc::new(base.finish()),
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
impl TableProvider for GoodsProvider {
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
