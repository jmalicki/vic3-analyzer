//! `alerts()` / `alerts('all')` → rows from `vic3-prices::alerts` (`docs/sql.md`).
//!
//! Zero-arg form is player-scoped: keep rows with `state_id` NULL or owned by
//! [`World::player_tag`](vic3_prices::World::player_tag) (strict; no first-country
//! fallback). `alerts('all')` is the unfiltered save-wide set.
//!
//! Projection / filter / LIMIT (speedup D, issue #37):
//! - Mitigations builders run only when the `mitigations` column is projected
//!   (or `SELECT *`). Projecting `evidence` alone stays on the lean path.
//! - Exact equality on `severity`, `kind`, `good_id`, `state_id`, `building_id`,
//!   and `id` is applied in-provider before mitigations.
//! - `LIMIT` truncates before mitigations when every filter is Exact (or there
//!   are no filters). Residual Unsupported filters disable early LIMIT.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{Int32Builder, RecordBatch, StringBuilder, UInt32Builder};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::{Session, TableFunctionImpl};
use datafusion::common::{plan_err, Result as DfResult};
use datafusion::datasource::{TableProvider, TableType};
use datafusion::logical_expr::{Expr, TableProviderFilterPushDown};
use datafusion::physical_plan::ExecutionPlan;
use vic3_prices::{Alert, AlertKind};

use crate::binding::{projection_includes, SessionBinding};
use crate::exec::memory_exec;
use crate::filter::Pred;
use crate::providers::pushdown::{matches_i32, matches_str, matches_u32, PushSupport};
use crate::schema::alerts_schema;
use crate::scope::player_owned_state_ids;
use crate::udfs::literal_str;

/// Column index of `mitigations` in [`alerts_schema`] (evidence is 8 and free).
const ALERTS_MITIGATIONS_COL: usize = 9;

const ALERTS_PUSH: PushSupport = PushSupport {
    eq_u32: &["state_id", "building_id"],
    eq_i32: &["severity"],
    eq_str: &["kind", "good_id", "id"],
    range_str: &[],
};

/// TVF wrapping [`SessionBinding::alerts`] for the bound session.
#[derive(Debug)]
pub struct AlertsTvf {
    binding: Arc<SessionBinding>,
}

impl AlertsTvf {
    pub fn new(binding: Arc<SessionBinding>) -> Self {
        Self { binding }
    }
}

impl TableFunctionImpl for AlertsTvf {
    fn call(&self, args: &[Expr]) -> DfResult<Arc<dyn TableProvider>> {
        let all = match args {
            [] => false,
            [arg] => {
                let s = literal_str(arg, 1)?;
                if s == "all" {
                    true
                } else {
                    return plan_err!("alerts() accepts no arguments or alerts('all'); got {s:?}");
                }
            }
            _ => {
                return plan_err!("alerts() accepts no arguments or alerts('all')");
            }
        };
        Ok(Arc::new(AlertsProvider {
            binding: Arc::clone(&self.binding),
            schema: alerts_schema(),
            all,
        }))
    }
}

#[derive(Debug)]
struct AlertsProvider {
    binding: Arc<SessionBinding>,
    schema: SchemaRef,
    /// When true, emit every alert; when false, player-scope by `state_id`.
    all: bool,
}

impl AlertsProvider {
    fn batch(
        &self,
        with_mitigations: bool,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DfResult<RecordBatch> {
        let preds = ALERTS_PUSH.collect_preds(filters);
        let early_limit = filters.is_empty()
            || filters.iter().all(|f| {
                matches!(
                    ALERTS_PUSH.classify(&[f]).as_slice(),
                    [TableProviderFilterPushDown::Exact]
                )
            });

        let lean = self.binding.alerts(false);
        let mut ordered_ids: Vec<String> = lean
            .alerts
            .iter()
            .filter(|alert| self.in_scope(alert) && alert_matches(alert, &preds))
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

        // Unfiltered full-save SELECT * — warm the shared fat cache.
        if self.all && preds.is_empty() && early_limit && limit.is_none() {
            let fat = self.binding.alerts(true);
            return self.emit_by_ids(fat.as_ref(), &ordered_ids);
        }

        let id_set: BTreeSet<String> = ordered_ids.iter().cloned().collect();
        let fat = self.binding.alerts_mitigating(id_set);
        self.emit_by_ids(&fat, &ordered_ids)
    }

    fn in_scope(&self, alert: &Alert) -> bool {
        if self.all {
            return true;
        }
        let player_states = player_owned_state_ids(self.binding.world.as_ref());
        match alert.state_id {
            None => true,
            Some(id) => player_states.as_ref().is_some_and(|ids| ids.contains(&id)),
        }
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
        self.emit_refs(&alerts)
    }

    fn emit_refs(&self, alerts: &[&Alert]) -> DfResult<RecordBatch> {
        let mut id = StringBuilder::new();
        let mut kind = StringBuilder::new();
        let mut severity = Int32Builder::new();
        let mut title = StringBuilder::new();
        let mut summary = StringBuilder::new();
        let mut state_id = UInt32Builder::new();
        let mut building_id = UInt32Builder::new();
        let mut good_id = StringBuilder::new();
        let mut evidence = StringBuilder::new();
        let mut mitigations = StringBuilder::new();

        for alert in alerts {
            id.append_value(&alert.id);
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
            match &alert.good_id {
                Some(v) => good_id.append_value(v),
                None => good_id.append_null(),
            }
            // JSON text matches AlertsResult nested arrays (see json-schema.md).
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
                Arc::new(id.finish()),
                Arc::new(kind.finish()),
                Arc::new(severity.finish()),
                Arc::new(title.finish()),
                Arc::new(summary.finish()),
                Arc::new(state_id.finish()),
                Arc::new(building_id.finish()),
                Arc::new(good_id.finish()),
                Arc::new(evidence.finish()),
                Arc::new(mitigations.finish()),
            ],
        )
        .map_err(Into::into)
    }
}

fn alert_matches(alert: &Alert, preds: &[Pred]) -> bool {
    if !matches_i32(preds, "severity", i32::from(alert.severity)) {
        return false;
    }
    if !matches_str(preds, "kind", alert_kind_str(alert.kind)) {
        return false;
    }
    if !matches_str(preds, "id", &alert.id) {
        return false;
    }
    // Nullable columns: equality excludes NULL rows (SQL NULL ≠ value).
    if preds
        .iter()
        .any(|p| matches!(p, Pred::EqStr { column, .. } if column == "good_id"))
    {
        match &alert.good_id {
            Some(g) if matches_str(preds, "good_id", g) => {}
            _ => return false,
        }
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

#[async_trait]
impl TableProvider for AlertsProvider {
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
        Ok(ALERTS_PUSH.classify(filters))
    }

    async fn scan(
        &self,
        _state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DfResult<Arc<dyn ExecutionPlan>> {
        let with_mitigations = projection_includes(projection, ALERTS_MITIGATIONS_COL);
        memory_exec(self.batch(with_mitigations, filters, limit)?, projection)
    }
}

/// Snake_case `kind` strings for SQL (`docs/sql.md` / alerts JSON).
pub(crate) fn alert_kind_str(kind: AlertKind) -> &'static str {
    match kind {
        AlertKind::ElectricityShortage => "electricity_shortage",
        AlertKind::TransportationShortage => "transportation_shortage",
        AlertKind::GoodsShortage => "goods_shortage",
        AlertKind::NeedsUnmet => "needs_unmet",
        AlertKind::LowMarketAccess => "low_market_access",
        AlertKind::UnfilledEducation => "unfilled_education",
        AlertKind::UnfilledPops => "unfilled_pops",
        AlertKind::Underemployed => "underemployed",
    }
}
