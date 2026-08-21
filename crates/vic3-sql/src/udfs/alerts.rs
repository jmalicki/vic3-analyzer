//! `alerts()` / `alerts('all')` → rows from `vic3-prices::alerts` (`docs/sql.md`).
//!
//! Zero-arg form is player-scoped: keep rows with `state_id` NULL or owned by
//! [`World::player_tag`](vic3_prices::World::player_tag) (strict; no first-country
//! fallback). `alerts('all')` is the unfiltered save-wide set.

use std::any::Any;
use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{Int32Builder, RecordBatch, StringBuilder, UInt32Builder};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::{Session, TableFunctionImpl};
use datafusion::common::{plan_err, Result as DfResult};
use datafusion::datasource::{TableProvider, TableType};
use datafusion::logical_expr::Expr;
use datafusion::physical_plan::ExecutionPlan;
use vic3_prices::{Alert, AlertKind, World};

use crate::binding::{projection_needs_json, SessionBinding};
use crate::exec::memory_exec;
use crate::schema::alerts_schema;
use crate::udfs::literal_str;

/// Column indices for `evidence` / `mitigations` in [`alerts_schema`].
const ALERTS_JSON_COLS: &[usize] = &[8, 9];

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
    fn batch(&self, with_mitigations: bool, limit: Option<usize>) -> DfResult<RecordBatch> {
        let result = self.binding.alerts(with_mitigations);
        let scoped: Vec<&Alert> = if self.all {
            result.alerts.iter().collect()
        } else {
            let player_states = player_owned_state_ids(self.binding.world.as_ref());
            result
                .alerts
                .iter()
                .filter(|alert| match alert.state_id {
                    None => true,
                    Some(id) => player_states.as_ref().is_some_and(|ids| ids.contains(&id)),
                })
                .collect()
        };
        let alerts = match limit {
            Some(n) => &scoped[..n.min(scoped.len())],
            None => scoped.as_slice(),
        };

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

#[async_trait]
impl TableProvider for AlertsProvider {
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
        limit: Option<usize>,
    ) -> DfResult<Arc<dyn ExecutionPlan>> {
        let with_mitigations = projection_needs_json(projection, ALERTS_JSON_COLS);
        memory_exec(self.batch(with_mitigations, limit)?, projection)
    }
}

/// State ids owned by [`World::player_tag`] (strict; no first-country fallback).
///
/// Mirrors `player_state_ids` in `vic3-prices` optimize, but only when
/// `player_tag` is set.
fn player_owned_state_ids(world: &World) -> Option<BTreeSet<u32>> {
    let tag = world.player_tag.as_deref()?;
    let country = world.country_by_tag(tag)?;
    let mut ids: BTreeSet<u32> = country.states.iter().copied().collect();
    for state in &world.states {
        if state.country == Some(country.id) {
            ids.insert(state.id);
        }
    }
    Some(ids)
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
