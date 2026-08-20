//! Read-only `saves` catalog table (`docs/sql.md`).
//!
//! Rows come from [`crate::host::HostState`]'s in-memory catalog; `loaded` is
//! true only for the entry identity currently bound by `use_save`.

use std::any::Any;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use datafusion::arrow::array::{
    BooleanBuilder, RecordBatch, StringBuilder, TimestampMillisecondBuilder,
};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::Session;
use datafusion::datasource::{TableProvider, TableType};
use datafusion::error::Result as DfResult;
use datafusion::logical_expr::{Expr, TableProviderFilterPushDown};
use datafusion::physical_plan::ExecutionPlan;

use crate::exec::memory_exec;
use crate::host::HostState;
use crate::schema::saves_schema;

/// Table provider for `SELECT … FROM saves`.
#[derive(Debug)]
pub struct SavesProvider {
    host: Arc<HostState>,
    schema: SchemaRef,
}

impl SavesProvider {
    pub(crate) fn new(host: Arc<HostState>) -> Self {
        Self {
            host,
            schema: saves_schema(),
        }
    }

    fn batch(&self) -> DfResult<RecordBatch> {
        let entries = self.host.catalog_entries();
        let active = self.host.active_meta();
        let mut name = StringBuilder::new();
        let mut kind = StringBuilder::new();
        let mut mtime = TimestampMillisecondBuilder::new();
        let mut in_game_date = StringBuilder::new();
        let mut country = StringBuilder::new();
        let mut location = StringBuilder::new();
        let mut loaded = BooleanBuilder::new();

        for entry in &entries {
            name.append_value(&entry.name);
            kind.append_value(entry.kind.as_str());
            mtime.append_value(system_time_millis(entry.mtime));
            match &entry.in_game_date {
                Some(d) => in_game_date.append_value(d),
                None => in_game_date.append_null(),
            }
            match &entry.country {
                Some(c) => country.append_value(c),
                None => country.append_null(),
            }
            location.append_value(entry.location.as_str());
            // Identity without path: agent-facing columns only (name/location/mtime).
            let is_loaded = active.as_ref().is_some_and(|a| {
                a.entry.name == entry.name
                    && a.entry.location == entry.location
                    && a.entry.mtime == entry.mtime
            });
            loaded.append_value(is_loaded);
        }

        RecordBatch::try_new(
            Arc::clone(&self.schema),
            vec![
                Arc::new(name.finish()),
                Arc::new(kind.finish()),
                Arc::new(mtime.finish()),
                Arc::new(in_game_date.finish()),
                Arc::new(country.finish()),
                Arc::new(location.finish()),
                Arc::new(loaded.finish()),
            ],
        )
        .map_err(Into::into)
    }
}

fn system_time_millis(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[async_trait]
impl TableProvider for SavesProvider {
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
        Ok(vec![
            TableProviderFilterPushDown::Unsupported;
            filters.len()
        ])
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
