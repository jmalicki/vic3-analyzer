use std::any::Any;
use std::collections::HashMap;
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
use crate::schema::states_schema;
use crate::scope::{state_in_scope, TableScope};

use super::pushdown::{matches_str, matches_u32, PushSupport};

const PUSH: PushSupport = PushSupport {
    eq_u32: &["state_id"],
    eq_i32: &[],
    eq_str: &["region_id", "region_name", "owner_tag"],
    range_str: &[],
};

#[derive(Debug)]
pub struct StatesProvider {
    binding: Arc<SessionBinding>,
    scope: TableScope,
    schema: SchemaRef,
    by_id: HashMap<u32, usize>,
    by_region_id: HashMap<String, Vec<usize>>,
    by_region_name: HashMap<String, Vec<usize>>,
}

impl StatesProvider {
    pub fn new(binding: Arc<SessionBinding>, scope: TableScope) -> Self {
        let mut rows: Vec<_> = binding.prices.states.iter().collect();
        rows.sort_by_key(|s| s.id);
        let mut by_id = HashMap::new();
        let mut by_region_id: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_region_name: HashMap<String, Vec<usize>> = HashMap::new();
        // Indexes are rebuilt against sorted order in scan; store id→presence only.
        for (i, s) in binding.prices.states.iter().enumerate() {
            by_id.insert(s.id, i);
            if let Some(r) = &s.region_id {
                by_region_id.entry(r.clone()).or_default().push(i);
            }
            if let Some(r) = &s.region_name {
                by_region_name.entry(r.clone()).or_default().push(i);
            }
        }
        Self {
            binding,
            scope,
            schema: states_schema(),
            by_id,
            by_region_id,
            by_region_name,
        }
    }

    fn owner_tag(&self, country_id: Option<u32>) -> Option<&str> {
        let id = country_id?;
        self.binding
            .prices
            .countries
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.tag.as_str())
    }

    fn batch(&self, filters: &[Expr]) -> DfResult<RecordBatch> {
        let preds = PUSH.collect_preds(filters);
        let mut ids = UInt32Builder::new();
        let mut region_id = StringBuilder::new();
        let mut region_name = StringBuilder::new();
        let mut owner_tag = StringBuilder::new();
        let mut market_id = UInt32Builder::new();
        let mut infrastructure = Float64Builder::new();
        let mut arable_land = Float64Builder::new();

        let mut rows: Vec<_> = self.binding.prices.states.iter().collect();
        rows.sort_by_key(|s| s.id);

        // Exact equality via HashMap when a single eq predicate is present.
        let selected: Vec<&vic3_prices::StateInfo> = if let Some(crate::filter::Pred::EqU32 {
            column,
            value,
        }) = preds.iter().find(
            |p| matches!(p, crate::filter::Pred::EqU32 { column, .. } if column == "state_id"),
        ) {
            let _ = column;
            match self.by_id.get(value) {
                Some(&i) => vec![&self.binding.prices.states[i]],
                None => Vec::new(),
            }
        } else if let Some(crate::filter::Pred::EqStr { column, value }) = preds.iter().find(
            |p| matches!(p, crate::filter::Pred::EqStr { column, .. } if column == "region_id"),
        ) {
            let _ = column;
            self.by_region_id
                .get(value)
                .map(|ix| ix.iter().map(|&i| &self.binding.prices.states[i]).collect())
                .unwrap_or_default()
        } else if let Some(crate::filter::Pred::EqStr { column, value }) = preds.iter().find(
            |p| matches!(p, crate::filter::Pred::EqStr { column, .. } if column == "region_name"),
        ) {
            let _ = column;
            self.by_region_name
                .get(value)
                .map(|ix| ix.iter().map(|&i| &self.binding.prices.states[i]).collect())
                .unwrap_or_default()
        } else {
            rows
        };

        for s in selected {
            if !state_in_scope(self.scope, self.binding.world.as_ref(), Some(s.id)) {
                continue;
            }
            let tag = self.owner_tag(s.country_id);
            if !matches_u32(&preds, "state_id", s.id) {
                continue;
            }
            if let Some(r) = &s.region_id {
                if !matches_str(&preds, "region_id", r) {
                    continue;
                }
            } else if preds.iter().any(
                |p| matches!(p, crate::filter::Pred::EqStr { column, .. } if column == "region_id"),
            ) {
                continue;
            }
            if let Some(r) = &s.region_name {
                if !matches_str(&preds, "region_name", r) {
                    continue;
                }
            } else if preds.iter().any(|p| {
                matches!(p, crate::filter::Pred::EqStr { column, .. } if column == "region_name")
            }) {
                continue;
            }
            if let Some(t) = tag {
                if !matches_str(&preds, "owner_tag", t) {
                    continue;
                }
            } else if preds.iter().any(
                |p| matches!(p, crate::filter::Pred::EqStr { column, .. } if column == "owner_tag"),
            ) {
                continue;
            }

            ids.append_value(s.id);
            append_opt_str(&mut region_id, s.region_id.as_deref());
            append_opt_str(&mut region_name, s.region_name.as_deref());
            append_opt_str(&mut owner_tag, tag);
            append_opt_u32(&mut market_id, s.market_id);
            append_opt_f64(&mut infrastructure, s.infrastructure);
            append_opt_f64(&mut arable_land, s.arable_land);
        }

        RecordBatch::try_new(
            Arc::clone(&self.schema),
            vec![
                Arc::new(ids.finish()),
                Arc::new(region_id.finish()),
                Arc::new(region_name.finish()),
                Arc::new(owner_tag.finish()),
                Arc::new(market_id.finish()),
                Arc::new(infrastructure.finish()),
                Arc::new(arable_land.finish()),
            ],
        )
        .map_err(Into::into)
    }
}

fn append_opt_str(b: &mut StringBuilder, v: Option<&str>) {
    match v {
        Some(s) => b.append_value(s),
        None => b.append_null(),
    }
}

fn append_opt_u32(b: &mut UInt32Builder, v: Option<u32>) {
    match v {
        Some(n) => b.append_value(n),
        None => b.append_null(),
    }
}

fn append_opt_f64(b: &mut Float64Builder, v: Option<f64>) {
    match v {
        Some(n) => b.append_value(n),
        None => b.append_null(),
    }
}

#[async_trait]
impl TableProvider for StatesProvider {
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
