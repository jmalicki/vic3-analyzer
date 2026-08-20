//! [`SqlEngine`]: DataFusion [`SessionContext`] + catalog / session binding.
//!
//! Two construction paths: [`SqlEngine::bind`] (in-memory snapshot, no catalog)
//! and [`SqlEngine::with_catalog`] (host `use_save` + `saves` / `latest.*`).

use std::sync::Arc;

use datafusion::arrow::array::RecordBatch;
use datafusion::execution::config::SessionConfig;
use datafusion::prelude::SessionContext;
use vic3_catalog::{SaveCatalog, SaveRoot};
use vic3_defs::GameDefs;
use vic3_prices::{PricesResult, World};

use crate::binding::SessionBinding;
use crate::host::{ActiveMeta, EngineLoadOpts, HostState, UseSaveRequest, UseSaveResult};
use crate::providers;
use crate::readonly::assert_readonly;
use crate::SqlError;

/// Read-only SQL engine over catalog + optional active analysis snapshot.
pub struct SqlEngine {
    ctx: SessionContext,
    /// Direct bind path (no catalog).
    binding: Option<Arc<SessionBinding>>,
    /// Present when constructed with [`Self::with_catalog`].
    host: Option<Arc<HostState>>,
}

impl SqlEngine {
    /// Bind `defs` / `world` / `prices` and register fact tables (no catalog).
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
        Ok(Self {
            ctx,
            binding: Some(binding),
            host: None,
        })
    }

    /// Open with a save catalog; `active.*` / unqualified facts need [`Self::use_save`].
    ///
    /// `latest.*` and `saves` are queryable immediately. Binding is never done
    /// via SQL mutation.
    pub async fn with_catalog(
        catalog: SaveCatalog,
        load: EngineLoadOpts,
    ) -> Result<Self, SqlError> {
        let host = Arc::new(HostState::new(catalog, load));
        let config = SessionConfig::new().with_information_schema(true);
        let ctx = SessionContext::new_with_config(config);
        providers::register_catalog_host(&ctx, Arc::clone(&host)).await?;
        Ok(Self {
            ctx,
            binding: None,
            host: Some(host),
        })
    }

    /// Active binding (defs / world / prices), if `bind` ran (direct path).
    ///
    /// After [`Self::use_save`], prefer [`Self::active_binding`].
    pub fn binding(&self) -> Option<&SessionBinding> {
        self.binding.as_deref()
    }

    /// Owned active binding after `bind` / `use_save`.
    pub fn active_binding(&self) -> Option<Arc<SessionBinding>> {
        self.binding
            .clone()
            .or_else(|| self.host.as_ref().and_then(|h| h.active_binding()))
    }

    /// Underlying DataFusion context (for advanced callers).
    pub fn context(&self) -> &SessionContext {
        &self.ctx
    }

    /// Rescan allowlisted save roots and invalidate the `latest.*` cache.
    pub fn refresh_catalog(&self, roots: &[SaveRoot]) -> Result<usize, SqlError> {
        let host = self.host.as_ref().ok_or_else(|| {
            SqlError::internal("refresh_catalog requires SqlEngine::with_catalog")
        })?;
        host.refresh_catalog(roots)
    }

    /// Host API: bind session by stub or selector (`docs/mcp.md`).
    ///
    /// Loads via `vic3-api`, installs the process analysis session, and
    /// rebinds `active.*` / unqualified fact tables. Never a SQL mutation.
    pub async fn use_save(&self, req: UseSaveRequest) -> Result<UseSaveResult, SqlError> {
        let host = self
            .host
            .as_ref()
            .ok_or_else(|| SqlError::internal("use_save requires SqlEngine::with_catalog"))?;
        let entry = host.resolve_request(&req)?;
        let loaded = host.load_entry(&entry, true)?;

        let mut meta_entry = entry.clone();
        meta_entry.in_game_date = loaded.in_game_date.clone();
        meta_entry.country = loaded.country.clone();
        host.set_active(ActiveMeta {
            entry: meta_entry.clone(),
            binding: Arc::clone(&loaded.binding),
        });

        providers::register_all(&self.ctx, Arc::clone(&loaded.binding)).await?;
        crate::udfs::register(&self.ctx, Arc::clone(&loaded.binding))?;

        Ok(UseSaveResult {
            name: meta_entry.name,
            kind: meta_entry.kind.as_str().to_string(),
            in_game_date: loaded.in_game_date,
            country: loaded.country,
            loaded: true,
        })
    }

    /// Parse + enforce read-only, then collect result batches.
    pub async fn query(&self, sql: &str) -> Result<Vec<RecordBatch>, SqlError> {
        assert_readonly(sql)?;
        let df = self.ctx.sql(sql).await?;
        Ok(df.collect().await?)
    }
}
