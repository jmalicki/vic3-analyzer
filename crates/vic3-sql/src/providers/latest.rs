//! `latest.*` fact tables: resolve max-mtime save at read time without mutating session.
//!
//! Distinct from `active.*`: a `SELECT` here must not call `use_save` or install
//! the process analysis session (`docs/sql.md`).

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
use crate::scope::TableScope;
use crate::SqlError;

/// Lazy view over the catalog's max-mtime save for one fact table.
///
/// On scan, ensures a cached binding via the host `latest.*` path
/// (`install = false`) then delegates to the concrete fact provider.
#[derive(Debug)]
pub struct LatestFactProvider {
    host: Arc<HostState>,
    table: FactTable,
    scope: TableScope,
    schema: SchemaRef,
}

impl LatestFactProvider {
    pub(crate) fn new(host: Arc<HostState>, table: FactTable, scope: TableScope) -> Self {
        Self {
            host,
            schema: table.schema(),
            table,
            scope,
        }
    }
}

#[async_trait]
impl TableProvider for LatestFactProvider {
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
        let inner = providers::provider_for(self.table, binding, self.scope);
        inner.scan(state, projection, filters, limit).await
    }
}

/// Placeholder for `active.*` / unqualified facts until `use_save` / `bind`.
///
/// Schema is still advertised so information_schema and planning see columns;
/// scans fail with [`SqlError::Unbound`].
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
