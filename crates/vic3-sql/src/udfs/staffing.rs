//! `building_staffing(state_id)` → building×profession gaps for one state.
//!
//! Arithmetic mirrors `vic3-prices` employment-alert staffing
//! (`level / staffing` scales employed counts to jobs-at-full-level).

use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::array::{Float64Builder, RecordBatch, StringBuilder, UInt32Builder};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::{Session, TableFunctionImpl};
use datafusion::common::{plan_err, Result as DfResult};
use datafusion::datasource::{TableProvider, TableType};
use datafusion::logical_expr::Expr;
use datafusion::physical_plan::ExecutionPlan;
use vic3_prices::{BuildingEconomics, ORDER_EPS};

use crate::binding::SessionBinding;
use crate::exec::memory_exec;
use crate::schema::building_staffing_schema;
use crate::udfs::args::literal_i64;

/// `building_staffing(state_id)` TVF over the bound session.
#[derive(Debug)]
pub struct BuildingStaffingTvf {
    binding: Arc<SessionBinding>,
}

impl BuildingStaffingTvf {
    pub fn new(binding: Arc<SessionBinding>) -> Self {
        Self { binding }
    }
}

impl TableFunctionImpl for BuildingStaffingTvf {
    fn call(&self, args: &[Expr]) -> DfResult<Arc<dyn TableProvider>> {
        if args.len() != 1 {
            return plan_err!("building_staffing(state_id) expects exactly one argument");
        }
        let state_id = literal_i64(&args[0], "state_id")?;
        if state_id < 0 || state_id > i64::from(u32::MAX) {
            return plan_err!("state_id out of range");
        }
        Ok(Arc::new(BuildingStaffingProvider {
            binding: Arc::clone(&self.binding),
            schema: building_staffing_schema(),
            state_id: state_id as u32,
        }))
    }
}

#[derive(Debug)]
struct BuildingStaffingProvider {
    binding: Arc<SessionBinding>,
    schema: SchemaRef,
    state_id: u32,
}

impl BuildingStaffingProvider {
    fn batch(&self) -> DfResult<RecordBatch> {
        let mut building_id = UInt32Builder::new();
        let mut building_name = StringBuilder::new();
        let mut type_id = StringBuilder::new();
        let mut staffing = Float64Builder::new();
        let mut level = Float64Builder::new();
        let mut profession_id = StringBuilder::new();
        let mut profession_name = StringBuilder::new();
        let mut employed_here = Float64Builder::new();
        let mut jobs_here = Float64Builder::new();
        let mut missing_here = Float64Builder::new();
        let mut state_jobs = Float64Builder::new();
        let mut state_stock = Float64Builder::new();
        let mut state_shortage = Float64Builder::new();

        for building in &self.binding.prices.buildings {
            if building.state_id != Some(self.state_id) {
                continue;
            }
            let name = building_label(self.binding.as_ref(), building);
            // Scale current employed counts up to full-level job demand.
            let ratio = if building.staffing > ORDER_EPS {
                building.level / building.staffing
            } else {
                0.0
            };
            let mut wrote = false;
            for row in &building.employees {
                if row.count <= ORDER_EPS && ratio <= 0.0 {
                    continue;
                }
                let jobs = if ratio > 0.0 {
                    row.count * ratio
                } else {
                    row.count
                };
                let missing = (jobs - row.count).max(0.0);
                let qual = self.binding.prices.state_qualifications.iter().find(|q| {
                    q.state_id == self.state_id && q.profession_name == row.profession_name
                });

                building_id.append_value(building.id);
                building_name.append_value(&name);
                type_id.append_value(&building.type_id);
                staffing.append_value(building.staffing);
                level.append_value(building.level);
                profession_id.append_value(&row.profession_name);
                match row
                    .profession_label
                    .as_deref()
                    .or_else(|| self.binding.label(&row.profession_name))
                {
                    Some(n) => profession_name.append_value(n),
                    None => profession_name.append_null(),
                }
                employed_here.append_value(row.count);
                jobs_here.append_value(jobs);
                missing_here.append_value(missing);
                state_jobs.append_value(qual.map(|q| q.jobs).unwrap_or(0.0));
                // Prefer employable stock when present (same as alert expander).
                state_stock.append_value(
                    qual.map(|q| q.employable.unwrap_or(q.qualified))
                        .unwrap_or(0.0),
                );
                state_shortage.append_value(qual.map(|q| q.shortage).unwrap_or(0.0));
                wrote = true;
            }
            if !wrote {
                // Keep buildings visible when employee lists are empty.
                building_id.append_value(building.id);
                building_name.append_value(&name);
                type_id.append_value(&building.type_id);
                staffing.append_value(building.staffing);
                level.append_value(building.level);
                profession_id.append_null();
                profession_name.append_null();
                employed_here.append_null();
                jobs_here.append_null();
                missing_here.append_null();
                state_jobs.append_null();
                state_stock.append_null();
                state_shortage.append_null();
            }
        }

        RecordBatch::try_new(
            Arc::clone(&self.schema),
            vec![
                Arc::new(building_id.finish()),
                Arc::new(building_name.finish()),
                Arc::new(type_id.finish()),
                Arc::new(staffing.finish()),
                Arc::new(level.finish()),
                Arc::new(profession_id.finish()),
                Arc::new(profession_name.finish()),
                Arc::new(employed_here.finish()),
                Arc::new(jobs_here.finish()),
                Arc::new(missing_here.finish()),
                Arc::new(state_jobs.finish()),
                Arc::new(state_stock.finish()),
                Arc::new(state_shortage.finish()),
            ],
        )
        .map_err(Into::into)
    }
}

fn building_label(binding: &SessionBinding, building: &BuildingEconomics) -> String {
    binding
        .label(&building.type_id)
        .map(str::to_owned)
        .or_else(|| {
            binding
                .prices
                .building_types
                .iter()
                .find(|t| t.id == building.type_id)
                .and_then(|t| t.name.clone())
        })
        .unwrap_or_else(|| building.type_id.clone())
}

#[async_trait]
impl TableProvider for BuildingStaffingProvider {
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
