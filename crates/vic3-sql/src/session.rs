//! [`SqlEngine`]: DataFusion session context + catalog / session binding.
//!
//! # Construction
//!
//! | Path | When |
//! | --- | --- |
//! | [`SqlEngine::bind`] | Tests / in-process snapshot; no `saves` / `latest.*` |
//! | [`SqlEngine::with_catalog`] | Desktop + MCP; catalog host API |
//!
//! After `with_catalog`, query `saves` or `latest.*` freely; `active.*` and
//! unqualified facts error with [`crate::SqlError::Unbound`] until
//! [`SqlEngine::use_save`].

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
use vic3_catalog::SaveEntry;

/// Read-only SQL engine over catalog + optional active analysis snapshot.
///
/// Session binding is never performed by SQL. Hosts call [`Self::use_save`];
/// [`Self::query`] only accepts SELECT / WITH…SELECT / EXPLAIN.
pub struct SqlEngine {
    ctx: SessionContext,
    /// Direct bind path (no catalog).
    binding: Option<Arc<SessionBinding>>,
    /// Present when constructed with [`Self::with_catalog`].
    host: Option<Arc<HostState>>,
}

impl SqlEngine {
    /// Bind `defs` / `world` / `prices` and register fact tables + UDFs (no catalog).
    ///
    /// Unqualified names and `active.*` are immediately queryable. There is no
    /// `saves` table or `latest.*` namespace on this path.
    ///
    /// # Errors
    ///
    /// Returns [`SqlError`] if provider or UDF registration fails.
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
    /// Registers `saves`, unbound fact placeholders, and lazy `latest.*` views.
    /// Binding is never done via SQL mutation.
    ///
    /// # Errors
    ///
    /// Returns [`SqlError`] if catalog host registration fails.
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

    /// Active binding for the direct [`Self::bind`] path only.
    ///
    /// After [`Self::use_save`], prefer [`Self::active_binding`].
    pub fn binding(&self) -> Option<&SessionBinding> {
        self.binding.as_deref()
    }

    /// Owned active binding after `bind` / `use_save` (either construction path).
    pub fn active_binding(&self) -> Option<Arc<SessionBinding>> {
        self.binding
            .clone()
            .or_else(|| self.host.as_ref().and_then(|h| h.active_binding()))
    }

    /// Underlying DataFusion context (advanced callers / EXPLAIN debugging).
    pub fn context(&self) -> &SessionContext {
        &self.ctx
    }

    /// Rescan allowlisted save roots and invalidate the `latest.*` cache.
    ///
    /// # Errors
    ///
    /// [`SqlError::Internal`] if this engine was built with [`Self::bind`]
    /// (no catalog). Propagates catalog I/O failures otherwise.
    pub fn refresh_catalog(&self, roots: &[SaveRoot]) -> Result<usize, SqlError> {
        let host = self.host.as_ref().ok_or_else(|| {
            SqlError::internal("refresh_catalog requires SqlEngine::with_catalog")
        })?;
        host.refresh_catalog(roots)
    }

    /// Current catalog snapshot (agent-facing fields; absolute paths may be present).
    ///
    /// # Errors
    ///
    /// [`SqlError::Internal`] without a catalog host.
    pub fn catalog_entries(&self) -> Result<Vec<SaveEntry>, SqlError> {
        let host = self.host.as_ref().ok_or_else(|| {
            SqlError::internal("catalog_entries requires SqlEngine::with_catalog")
        })?;
        Ok(host.catalog_entries())
    }

    /// Active session metadata after [`Self::use_save`], if any.
    pub fn active_session(&self) -> Option<ActiveSessionInfo> {
        self.host.as_ref().and_then(|h| {
            h.active_meta().map(|meta| ActiveSessionInfo {
                name: meta.entry.name,
                kind: meta.entry.kind.as_str().to_string(),
                in_game_date: meta.entry.in_game_date,
                country: meta.entry.country,
                loaded: true,
                location: meta.entry.location.as_str().to_string(),
            })
        })
    }

    /// Host API: bind session by stub or selector (`docs/mcp.md` / `docs/sql.md`).
    ///
    /// Loads via `vic3-api` with `install = true` (process analysis session),
    /// then rebinds `active.*` / unqualified fact tables and UDFs. Never a SQL
    /// mutation.
    ///
    /// # Arguments
    ///
    /// * `req` — exactly one of [`UseSaveRequest::name`] or
    ///   [`UseSaveRequest::selector`]; optional `location` / `mtime` disambiguate
    ///   stub collisions across roots.
    ///
    /// # Errors
    ///
    /// - [`SqlError::Internal`] if built without catalog
    /// - [`SqlError::NotFound`] / [`SqlError::Ambiguous`] on stub resolution
    /// - [`SqlError::Api`] / [`SqlError::Catalog`] on load failures
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
        host.patch_catalog_meta(&meta_entry);
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
            country_id: loaded.country_id,
            market_id: loaded.market_id,
            loaded: true,
        })
    }

    /// Parse + enforce read-only, then collect result batches.
    ///
    /// # Errors
    ///
    /// [`SqlError::ReadOnly`] before planning; [`SqlError::Unbound`] when scanning
    /// unbound facts; DataFusion planning/execution as [`SqlError::DataFusion`].
    pub async fn query(&self, sql: &str) -> Result<Vec<RecordBatch>, SqlError> {
        assert_readonly(sql)?;
        let df = self.ctx.sql(sql).await?;
        Ok(df.collect().await?)
    }
}

/// Agent-facing snapshot of the bound save (`vic3://session`, `use_save` result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveSessionInfo {
    /// Filename stub (primary handle).
    pub name: String,
    /// `autosave` \| `named` \| `ironman` \| …
    pub kind: String,
    pub in_game_date: Option<String>,
    pub country: Option<String>,
    /// Always `true` when this struct is produced from an active bind.
    pub loaded: bool,
    /// `local` \| `steam_cloud`.
    pub location: String,
}
