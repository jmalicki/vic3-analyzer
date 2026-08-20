//! `latest.*` fact tables: resolve max-mtime save at read time without mutating session.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::Session;
use datafusion::datasource::{TableProvider, TableType};
use datafusion::error::{DataFusionError, Result as DfResult};
use datafusion::logical_expr::{Expr, TableProviderFilterPushDown};
use datafusion::physical_plan::ExecutionPlan;

use crate::host::HostState;
use crate::providers::{self, FactTable};
use crate::SqlError;

#[derive(Debug)]
pub struct LatestFactProvider {
    host: Arc<HostState>,
    table: FactTable,
    schema: SchemaRef,
}

impl LatestFactProvider {
    pub fn new(host: Arc<HostState>, table: FactTable) -> Self {
        Self {
            host,
            schema: table.schema(),
            table,
        }
    }
}

#[async_trait]
impl TableProvider for LatestFactProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn table_type(&self) -> TableType {
        TableType::View
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DfResult<Vec<TableProviderFilterPushDown>> {
        // Delegate unknown — underlying provider decides after load.
        Ok(vec![TableProviderFilterPushDown::Inexact; filters.len()])
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DfResult<Arc<dyn ExecutionPlan>> {
        let binding = self.host.ensure_latest_binding().map_err(|e| {
            DataFusionError::External(Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        })?;
        let inner = providers::provider_for(self.table, binding);
        inner.scan(state, projection, filters, limit).await
    }
}

/// Fact table that errors until `use_save` / `bind` installs a provider.
#[derive(Debug)]
pub struct UnboundFactProvider {
    schema: SchemaRef,
    table: FactTable,
}

impl UnboundFactProvider {
    pub fn new(table: FactTable) -> Self {
        Self {
            schema: table.schema(),
            table,
        }
    }
}

#[async_trait]
impl TableProvider for UnboundFactProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn table_type(&self) -> TableType {
        TableType::View
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
        _projection: Option<&Vec<usize>>,
        _filters: &[Expr],
        _limit: Option<usize>,
    ) -> DfResult<Arc<dyn ExecutionPlan>> {
        let _ = self.table;
        Err(DataFusionError::External(
            Box::new(SqlError::Unbound) as Box<dyn std::error::Error + Send + Sync>
        ))
    }
}
