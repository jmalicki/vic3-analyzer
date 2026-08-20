//! `shortage_analysis(good)` → scarce-good alert rows plus market magnitudes.
//!
//! Uses [`SessionBinding::goods_shortage_alerts`] so education/pop collectors
//! are not run. `good = NULL` means all such alerts; a string literal filters
//! by script id. Buy/sell/shortage/price/base come from the market `goods` row.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{
    Float64Builder, Int32Builder, RecordBatch, StringBuilder, UInt32Builder,
};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::{Session, TableFunctionImpl};
use datafusion::common::{plan_err, Result as DfResult};
use datafusion::datasource::{TableProvider, TableType};
use datafusion::logical_expr::Expr;
use datafusion::physical_plan::ExecutionPlan;
use vic3_prices::AlertKind;

use crate::binding::{goods_shortage, projection_needs_json, SessionBinding};
use crate::exec::memory_exec;
use crate::schema::shortage_analysis_schema;
use crate::udfs::alerts::alert_kind_str;
use crate::udfs::args::literal_utf8;

/// Column indices for `evidence` / `mitigations` in [`shortage_analysis_schema`].
const SHORTAGE_JSON_COLS: &[usize] = &[13, 14];

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
    fn batch(&self, with_mitigations: bool, limit: Option<usize>) -> DfResult<RecordBatch> {
        let result = self.binding.goods_shortage_alerts(with_mitigations);

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

        let mut emitted = 0usize;
        for alert in &result.alerts {
            if !is_goods_shortage_kind(alert.kind) {
                continue;
            }
            let Some(good_id) = alert.good_id.as_deref() else {
                continue;
            };
            if let Some(filter) = &self.good {
                if good_id != filter {
                    continue;
                }
            }
            if let Some(n) = limit {
                if emitted >= n {
                    break;
                }
            }

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
            emitted += 1;
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
        let with_mitigations = projection_needs_json(projection, SHORTAGE_JSON_COLS);
        memory_exec(self.batch(with_mitigations, limit)?, projection)
    }
}
