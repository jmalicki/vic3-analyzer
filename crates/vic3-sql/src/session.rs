//! [`SqlEngine`]: DataFusion [`SessionContext`] + bound fact tables.

use std::sync::Arc;

use datafusion::arrow::array::RecordBatch;
use datafusion::execution::config::SessionConfig;
use datafusion::prelude::SessionContext;
use vic3_defs::GameDefs;
use vic3_prices::{PricesResult, World};

use crate::binding::SessionBinding;
use crate::providers;
use crate::readonly::assert_readonly;
use crate::SqlError;

/// Read-only SQL engine over one in-memory analysis snapshot.
pub struct SqlEngine {
    ctx: SessionContext,
    binding: Arc<SessionBinding>,
}

impl SqlEngine {
    /// Bind `defs` / `world` / `prices` and register fact tables.
    pub async fn bind(
        defs: GameDefs,
        world: World,
        prices: PricesResult,
    ) -> Result<Self, SqlError> {
        let binding = Arc::new(SessionBinding::new(defs, world, prices));
        let config = SessionConfig::new().with_information_schema(true);
        let ctx = SessionContext::new_with_config(config);
        providers::register_all(&ctx, Arc::clone(&binding)).await?;
        crate::udfs::register(&ctx, Arc::clone(&binding))?;
        Ok(Self { ctx, binding })
    }

    /// Active binding (defs / world / prices).
    pub fn binding(&self) -> &SessionBinding {
        &self.binding
    }

    /// Underlying DataFusion context (for advanced callers).
    pub fn context(&self) -> &SessionContext {
        &self.ctx
    }

    /// Parse + enforce read-only, then collect result batches.
    pub async fn query(&self, sql: &str) -> Result<Vec<RecordBatch>, SqlError> {
        assert_readonly(sql)?;
        let df = self.ctx.sql(sql).await?;
        Ok(df.collect().await?)
    }
}
