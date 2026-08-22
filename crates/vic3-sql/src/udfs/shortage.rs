//! `shortage_analysis(good)` → scarce-good alert rows plus market magnitudes.
//!
//! Uses [`SessionBinding::goods_shortage_alerts`] so education/pop collectors
//! are not run. `good = NULL` means all such alerts; a string literal filters
//! by script id. Buy/sell/shortage/price/base come from the market `goods` row.
//!
//! Same projection / Exact filter / LIMIT-before-mitigations policy as `alerts()`
//! (speedup D, issue #37).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{
    Float64Builder, Int32Builder, RecordBatch, StringBuilder, UInt32Builder,
};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::{Session, TableFunctionImpl};
use datafusion::common::{plan_err, Result as DfResult};
use datafusion::datasource::{TableProvider, TableType};
use datafusion::logical_expr::{Expr, TableProviderFilterPushDown};
use datafusion::physical_plan::ExecutionPlan;
use vic3_prices::{Alert, AlertKind};

use crate::binding::{goods_shortage, projection_includes, SessionBinding};
use crate::exec::memory_exec;
use crate::filter::Pred;
use crate::providers::pushdown::{matches_i32, matches_str, matches_u32, PushSupport};
use crate::schema::shortage_analysis_schema;
use crate::udfs::alerts::alert_kind_str;
use crate::udfs::args::literal_utf8;

/// Column index of `mitigations` in [`shortage_analysis_schema`] (evidence is 13).
const SHORTAGE_MITIGATIONS_COL: usize = 14;

const SHORTAGE_PUSH: PushSupport = PushSupport {
    eq_u32: &["state_id", "building_id"],
    eq_i32: &["severity"],
    eq_str: &["kind", "good", "alert_id"],
    range_str: &[],
};

/// `shortage_analysis(good)` TVF over the bound session.
#[derive(Debug)]
pub struct ShortageAnalysisTvf {
    binding: Arc<SessionBinding>,
}

impl ShortageAnalysisTvf {
    pub fn new(binding: Arc<SessionBinding>) -> Self {
        Self { binding }
    }
}

impl TableFunctionImpl for ShortageAnalysisTvf {
    fn call(&self, args: &[Expr]) -> DfResult<Arc<dyn TableProvider>> {
        if args.len() != 1 {
            return plan_err!("shortage_analysis(good) expects exactly one argument");
        }
        let good = literal_utf8(&args[0], "good")?;
        Ok(Arc::new(ShortageAnalysisProvider {
            binding: Arc::clone(&self.binding),
            schema: shortage_analysis_schema(),
            good,
        }))
    }
}

#[derive(Debug)]
struct ShortageAnalysisProvider {
    binding: Arc<SessionBinding>,
    schema: SchemaRef,
    /// `None` = all scarce-good alerts (electricity / transportation / goods).
    good: Option<String>,
}

impl ShortageAnalysisProvider {
    fn batch(
        &self,
        with_mitigations: bool,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DfResult<RecordBatch> {
        let preds = SHORTAGE_PUSH.collect_preds(filters);
        let early_limit = filters.is_empty()
            || filters.iter().all(|f| {
                matches!(
                    SHORTAGE_PUSH.classify(&[f]).as_slice(),
                    [TableProviderFilterPushDown::Exact]
                )
            });

        let lean = self.binding.goods_shortage_alerts(false);
        let mut ordered_ids: Vec<String> = lean
            .alerts
            .iter()
            .filter(|alert| self.row_ok(alert, &preds))
            .map(|alert| alert.id.clone())
            .collect();

        if early_limit {
            if let Some(n) = limit {
                ordered_ids.truncate(n.min(ordered_ids.len()));
            }
        }

        if !with_mitigations {
            return self.emit_by_ids(lean.as_ref(), &ordered_ids);
        }

        if self.good.is_none() && preds.is_empty() && early_limit && limit.is_none() {
            let fat = self.binding.goods_shortage_alerts(true);
            return self.emit_by_ids(fat.as_ref(), &ordered_ids);
        }

        let id_set: BTreeSet<String> = ordered_ids.iter().cloned().collect();
        let fat = self.binding.goods_shortage_alerts_mitigating(id_set);
        self.emit_by_ids(&fat, &ordered_ids)
    }

    fn row_ok(&self, alert: &Alert, preds: &[Pred]) -> bool {
        if !is_goods_shortage_kind(alert.kind) {
            return false;
        }
        let Some(good_id) = alert.good_id.as_deref() else {
            return false;
        };
        if let Some(filter) = &self.good {
            if good_id != filter {
                return false;
            }
        }
        shortage_matches(alert, good_id, preds)
    }

    fn emit_by_ids(
        &self,
        result: &vic3_prices::AlertsResult,
        ordered_ids: &[String],
    ) -> DfResult<RecordBatch> {
        let by_id: BTreeMap<&str, &Alert> =
            result.alerts.iter().map(|a| (a.id.as_str(), a)).collect();
        let alerts: Vec<&Alert> = ordered_ids
            .iter()
            .filter_map(|id| by_id.get(id.as_str()).copied())
            .collect();

        let mut good_col = StringBuilder::new();
        let mut alert_id = StringBuilder::new();
        let mut kind = StringBuilder::new();
        let mut severity = Int32Builder::new();
        let mut title = StringBuilder::new();
        let mut summary = StringBuilder::new();
        let mut state_id = UInt32Builder::new();
        let mut building_id = UInt32Builder::new();
        let mut buy = Float64Builder::new();
        let mut sell = Float64Builder::new();
        let mut shortage = Float64Builder::new();
        let mut price = Float64Builder::new();
        let mut base = Float64Builder::new();
        let mut evidence = StringBuilder::new();
        let mut mitigations = StringBuilder::new();

        for alert in alerts {
            let good_id = alert.good_id.as_deref().unwrap_or("");
            let market = self.binding.prices.goods.iter().find(|g| g.id == good_id);

            good_col.append_value(good_id);
            alert_id.append_value(&alert.id);
            kind.append_value(alert_kind_str(alert.kind));
            severity.append_value(i32::from(alert.severity));
            title.append_value(&alert.title);
            summary.append_value(&alert.summary);
            match alert.state_id {
                Some(v) => state_id.append_value(v),
                None => state_id.append_null(),
            }
            match alert.building_id {
                Some(v) => building_id.append_value(v),
                None => building_id.append_null(),
            }
            match market {
                Some(g) => {
                    buy.append_value(g.buy);
                    sell.append_value(g.sell);
                    // Same unmet-demand formula as goods fact tables (`docs/sql.md`).
                    shortage.append_value(goods_shortage(g.buy, g.sell));
                    price.append_value(g.price);
                    base.append_value(g.base);
                }
                None => {
                    buy.append_null();
                    sell.append_null();
                    shortage.append_null();
                    price.append_null();
                    base.append_null();
                }
            }
            evidence.append_value(
                serde_json::to_string(&alert.evidence).unwrap_or_else(|_| "[]".into()),
            );
            mitigations.append_value(
                serde_json::to_string(&alert.mitigations).unwrap_or_else(|_| "[]".into()),
            );
        }

        RecordBatch::try_new(
            Arc::clone(&self.schema),
            vec![
                Arc::new(good_col.finish()),
                Arc::new(alert_id.finish()),
                Arc::new(kind.finish()),
                Arc::new(severity.finish()),
                Arc::new(title.finish()),
                Arc::new(summary.finish()),
                Arc::new(state_id.finish()),
                Arc::new(building_id.finish()),
                Arc::new(buy.finish()),
                Arc::new(sell.finish()),
                Arc::new(shortage.finish()),
                Arc::new(price.finish()),
                Arc::new(base.finish()),
                Arc::new(evidence.finish()),
                Arc::new(mitigations.finish()),
            ],
        )
        .map_err(Into::into)
    }
}

fn shortage_matches(alert: &Alert, good_id: &str, preds: &[Pred]) -> bool {
    if !matches_i32(preds, "severity", i32::from(alert.severity)) {
        return false;
    }
    if !matches_str(preds, "kind", alert_kind_str(alert.kind)) {
        return false;
    }
    if !matches_str(preds, "good", good_id) {
        return false;
    }
    if !matches_str(preds, "alert_id", &alert.id) {
        return false;
    }
    if preds
        .iter()
        .any(|p| matches!(p, Pred::EqU32 { column, .. } if column == "state_id"))
    {
        match alert.state_id {
            Some(id) if matches_u32(preds, "state_id", id) => {}
            _ => return false,
        }
    }
    if preds
        .iter()
        .any(|p| matches!(p, Pred::EqU32 { column, .. } if column == "building_id"))
    {
        match alert.building_id {
            Some(id) if matches_u32(preds, "building_id", id) => {}
            _ => return false,
        }
    }
    true
}

fn is_goods_shortage_kind(kind: AlertKind) -> bool {
    matches!(
        kind,
        AlertKind::ElectricityShortage
            | AlertKind::TransportationShortage
            | AlertKind::GoodsShortage
    )
}

#[async_trait]
impl TableProvider for ShortageAnalysisProvider {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn table_type(&self) -> TableType {
        TableType::Temporary
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DfResult<Vec<TableProviderFilterPushDown>> {
        Ok(SHORTAGE_PUSH.classify(filters))
    }

    async fn scan(
        &self,
        _state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DfResult<Arc<dyn ExecutionPlan>> {
        let with_mitigations = projection_includes(projection, SHORTAGE_MITIGATIONS_COL);
        memory_exec(self.batch(with_mitigations, filters, limit)?, projection)
    }
}
