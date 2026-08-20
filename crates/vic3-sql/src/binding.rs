//! In-memory analysis snapshot bound into DataFusion providers and UDFs.
//!
//! Built by [`crate::SqlEngine::bind`] or by the catalog host after a successful
//! load (`use_save` / `latest.*`). Providers hold `Arc<SessionBinding>` — they
//! do not re-read the filesystem.

use std::sync::{Arc, Mutex};

use vic3_defs::GameDefs;
use vic3_prices::{
    alerts_with, goods_shortage_alerts, AlertsOptions, AlertsResult, PricesResult, World,
};

/// In-memory analysis snapshot bound into DataFusion providers.
#[derive(Debug)]
pub struct SessionBinding {
    pub defs: Arc<GameDefs>,
    pub world: Arc<World>,
    pub prices: Arc<PricesResult>,
    /// Cached full-detector alerts (with / without mitigations).
    alerts_cache: Mutex<AlertsCache>,
    /// Cached goods-shortage-only alerts (with / without mitigations).
    goods_alerts_cache: Mutex<AlertsCache>,
}

#[derive(Debug, Default)]
struct AlertsCache {
    with_mitigations: Option<Arc<AlertsResult>>,
    without_mitigations: Option<Arc<AlertsResult>>,
}

impl Clone for SessionBinding {
    fn clone(&self) -> Self {
        Self {
            defs: Arc::clone(&self.defs),
            world: Arc::clone(&self.world),
            prices: Arc::clone(&self.prices),
            // Fresh caches on clone — providers share via Arc, not Clone.
            alerts_cache: Mutex::new(AlertsCache::default()),
            goods_alerts_cache: Mutex::new(AlertsCache::default()),
        }
    }
}

impl SessionBinding {
    /// Wrap owned analysis pieces for provider/UDF sharing.
    pub fn new(defs: GameDefs, world: World, prices: PricesResult) -> Self {
        Self {
            defs: Arc::new(defs),
            world: Arc::new(world),
            prices: Arc::new(prices),
            alerts_cache: Mutex::new(AlertsCache::default()),
            goods_alerts_cache: Mutex::new(AlertsCache::default()),
        }
    }

    /// Localized label from defs, if present.
    pub fn label(&self, id: &str) -> Option<&str> {
        self.defs.labels.get(id).map(String::as_str)
    }

    /// Display name for a good script id (defs labels, else prices row name).
    pub fn good_name(&self, good_id: &str) -> Option<String> {
        self.defs.labels.get(good_id).cloned().or_else(|| {
            self.prices
                .goods
                .iter()
                .find(|g| g.id == good_id)
                .and_then(|g| g.name.clone())
        })
    }

    /// Full [`alerts_with`] result, cached per bind and mitigation mode.
    pub fn alerts(&self, with_mitigations: bool) -> Arc<AlertsResult> {
        cached_alerts(&self.alerts_cache, with_mitigations, || {
            alerts_with(
                self.world.as_ref(),
                self.defs.as_ref(),
                self.prices.as_ref(),
                AlertsOptions { with_mitigations },
            )
        })
    }

    /// Goods / electricity / transportation shortage alerts only.
    pub fn goods_shortage_alerts(&self, with_mitigations: bool) -> Arc<AlertsResult> {
        cached_alerts(&self.goods_alerts_cache, with_mitigations, || {
            goods_shortage_alerts(
                self.world.as_ref(),
                self.defs.as_ref(),
                self.prices.as_ref(),
                AlertsOptions { with_mitigations },
            )
        })
    }
}

fn cached_alerts(
    cache: &Mutex<AlertsCache>,
    with_mitigations: bool,
    compute: impl FnOnce() -> AlertsResult,
) -> Arc<AlertsResult> {
    {
        let guard = cache.lock().unwrap_or_else(|e| e.into_inner());
        let hit = if with_mitigations {
            guard.with_mitigations.as_ref()
        } else {
            guard.without_mitigations.as_ref()
        };
        if let Some(hit) = hit {
            return Arc::clone(hit);
        }
    }
    let computed = Arc::new(compute());
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if with_mitigations {
        guard.with_mitigations = Some(Arc::clone(&computed));
    } else {
        guard.without_mitigations = Some(Arc::clone(&computed));
    }
    computed
}

/// Shortage volumes for goods tables (`docs/sql.md` open question #1 — locked).
///
/// `max(0, buy − sell)` — unmet demand after sell orders. Not Paradox’s
/// shortage flag; aligns with qualification `shortage = max(0, jobs − stock)`.
pub fn goods_shortage(buy: f64, sell: f64) -> f64 {
    (buy - sell).max(0.0)
}

/// Whether a DataFusion projection includes heavy JSON columns.
///
/// `None` projection means all columns (mitigations required).
pub fn projection_needs_json(projection: Option<&Vec<usize>>, json_cols: &[usize]) -> bool {
    match projection {
        None => true,
        Some(cols) => cols.iter().any(|i| json_cols.contains(i)),
    }
}
