//! Build a single-partition [`ExecutionPlan`] from in-memory batches.

use std::sync::Arc;

use datafusion::arrow::array::RecordBatch;
use datafusion::arrow::compute::SortOptions;
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::datasource::memory::MemorySourceConfig;
use datafusion::error::Result as DfResult;
use datafusion::physical_expr::{LexOrdering, PhysicalSortExpr};
use datafusion::physical_plan::{ExecutionPlan, PhysicalExpr};

pub fn memory_exec(
    batch: RecordBatch,
    projection: Option<&Vec<usize>>,
) -> DfResult<Arc<dyn ExecutionPlan>> {
    let schema = batch.schema();
    let exec = MemorySourceConfig::try_new_exec(&[vec![batch]], schema, projection.cloned())?;
    Ok(exec)
}

/// Ascending sort key on `name` within `schema` (for EquivalenceProperties).
#[allow(dead_code)]
pub fn sort_by_name(schema: &SchemaRef, name: &str) -> Option<LexOrdering> {
    let idx = schema.index_of(name).ok()?;
    let expr = Arc::new(datafusion::physical_expr::expressions::Column::new(
        name, idx,
    )) as Arc<dyn PhysicalExpr>;
    LexOrdering::new(vec![PhysicalSortExpr {
        expr,
        options: SortOptions {
            descending: false,
            nulls_first: false,
        },
    }])
}
