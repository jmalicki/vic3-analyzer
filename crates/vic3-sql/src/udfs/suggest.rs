//! `suggest_mitigations([scope])` → one row per existing alert mitigation.
//!
//! # Semantics (important)
//!
//! This TVF **exposes** the heuristic mitigations already attached by
//! `vic3-prices::alerts` (often `extra_levels = 1`, ranked trade-center / build /
//! PM advice). It does **not** compute “size enough to fix” actions — no
//! solver loop that grows levels until the shortage clears. Sizing-to-fix is
//! an explicit future limitation; do not treat `extra_levels` as a clearance
//! quantity.
//!
//! # Scope
//!
//! Same player-vs-all rule as [`super::alerts`]:
//! - `suggest_mitigations()` / `suggest_mitigations('player')` — alerts whose
//!   `state_id` is NULL or owned by [`World::player_tag`](vic3_prices::World::player_tag)
//! - `suggest_mitigations('all')` — every alert in the save
//!
//! Args are plan-time string literals only (no subquery args in v1).

use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{RecordBatch, StringBuilder, UInt32Builder};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::{Session, TableFunctionImpl};
use datafusion::common::{plan_err, Result as DfResult};
use datafusion::datasource::{TableProvider, TableType};
use datafusion::logical_expr::Expr;
use datafusion::physical_plan::ExecutionPlan;
use vic3_prices::{Alert, Mitigation, MitigationAction};

use crate::binding::SessionBinding;
use crate::exec::memory_exec;
use crate::schema::suggest_mitigations_schema;
use crate::scope::player_owned_state_ids;
use crate::udfs::alerts::alert_kind_str;
use crate::udfs::literal_str;

/// TVF wrapping fat alerts + exploding [`Mitigation`] lists into rows.
#[derive(Debug)]
pub struct SuggestMitigationsTvf {
    binding: Arc<SessionBinding>,
}

impl SuggestMitigationsTvf {
    pub fn new(binding: Arc<SessionBinding>) -> Self {
        Self { binding }
    }
}

impl TableFunctionImpl for SuggestMitigationsTvf {
    fn call(&self, args: &[Expr]) -> DfResult<Arc<dyn TableProvider>> {
        let all = match args {
            [] => false,
            [arg] => {
                let s = literal_str(arg, 1)?;
                match s.as_str() {
                    "all" => true,
                    "player" => false,
                    other => {
                        return plan_err!(
                            "suggest_mitigations() accepts no arguments, \
                             suggest_mitigations('player'), or suggest_mitigations('all'); got {other:?}"
                        );
                    }
                }
            }
            _ => {
                return plan_err!(
                    "suggest_mitigations() accepts no arguments, \
                     suggest_mitigations('player'), or suggest_mitigations('all')"
                );
            }
        };
        Ok(Arc::new(SuggestMitigationsProvider {
            binding: Arc::clone(&self.binding),
            schema: suggest_mitigations_schema(),
            all,
        }))
    }
}

#[derive(Debug)]
struct SuggestMitigationsProvider {
    binding: Arc<SessionBinding>,
    schema: SchemaRef,
    /// When true, emit every mitigation; when false, player-scope by alert `state_id`.
    all: bool,
}

impl SuggestMitigationsProvider {
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

    fn batch(&self) -> DfResult<RecordBatch> {
        // Always need mitigation builders — this TVF exists to surface them.
        let result = self.binding.alerts(true);
        let mut cols = MitigationColumns::default();

        for alert in &result.alerts {
            if !self.in_scope(alert) {
                continue;
            }
            for mit in &alert.mitigations {
                cols.append(alert, mit);
            }
        }

        RecordBatch::try_new(Arc::clone(&self.schema), cols.finish()).map_err(Into::into)
    }
}

#[derive(Default)]
struct MitigationColumns {
    alert_id: StringBuilder,
    mitigation_id: StringBuilder,
    state_id: UInt32Builder,
    kind: StringBuilder,
    rank: UInt32Builder,
    action: StringBuilder,
    building: StringBuilder,
    good_id: StringBuilder,
    extra_levels: UInt32Builder,
    title: StringBuilder,
    detail: StringBuilder,
}

impl MitigationColumns {
    fn append(&mut self, alert: &Alert, mit: &Mitigation) {
        self.alert_id.append_value(&alert.id);
        self.mitigation_id.append_value(&mit.id);
        match alert.state_id {
            Some(v) => self.state_id.append_value(v),
            None => self.state_id.append_null(),
        }
        self.kind.append_value(alert_kind_str(alert.kind));
        self.rank.append_value(mit.rank);
        self.title.append_value(&mit.title);
        self.detail
            .append_value(serde_json::to_string(mit).unwrap_or_else(|_| "{}".into()));

        match &mit.action {
            Some(act) => {
                self.action.append_value(action_type_str(act));
                match act {
                    MitigationAction::Build {
                        building: b,
                        extra_levels: levels,
                        ..
                    } => {
                        self.building.append_value(b);
                        self.good_id.append_null();
                        match levels {
                            Some(n) => self.extra_levels.append_value(*n),
                            None => self.extra_levels.append_null(),
                        }
                    }
                    MitigationAction::FeederJob { building: b, .. } => {
                        self.building.append_value(b);
                        self.good_id.append_null();
                        self.extra_levels.append_null();
                    }
                    MitigationAction::TradeAlloc { good_id: g, .. }
                    | MitigationAction::SolGoods { good_id: g, .. } => {
                        self.building.append_null();
                        self.good_id.append_value(g);
                        self.extra_levels.append_null();
                    }
                    MitigationAction::Pm { .. } | MitigationAction::Subsidize { .. } => {
                        self.building.append_null();
                        self.good_id.append_null();
                        self.extra_levels.append_null();
                    }
                }
            }
            None => {
                self.action.append_null();
                self.building.append_null();
                self.good_id.append_null();
                self.extra_levels.append_null();
            }
        }
    }

    fn finish(mut self) -> Vec<Arc<dyn datafusion::arrow::array::Array>> {
        vec![
            Arc::new(self.alert_id.finish()),
            Arc::new(self.mitigation_id.finish()),
            Arc::new(self.state_id.finish()),
            Arc::new(self.kind.finish()),
            Arc::new(self.rank.finish()),
            Arc::new(self.action.finish()),
            Arc::new(self.building.finish()),
            Arc::new(self.good_id.finish()),
            Arc::new(self.extra_levels.finish()),
            Arc::new(self.title.finish()),
            Arc::new(self.detail.finish()),
        ]
    }
}

/// Snake_case `type` tag matching [`MitigationAction`] serde (`rename_all = "snake_case"`).
fn action_type_str(action: &MitigationAction) -> &'static str {
    match action {
        MitigationAction::Build { .. } => "build",
        MitigationAction::Pm { .. } => "pm",
        MitigationAction::Subsidize { .. } => "subsidize",
        MitigationAction::TradeAlloc { .. } => "trade_alloc",
        MitigationAction::FeederJob { .. } => "feeder_job",
        MitigationAction::SolGoods { .. } => "sol_goods",
    }
}

#[async_trait]
impl TableProvider for SuggestMitigationsProvider {
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
        memory_exec(self.batch()?, projection)
    }
}
