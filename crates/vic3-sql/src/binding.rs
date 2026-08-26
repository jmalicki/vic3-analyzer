//! In-memory analysis snapshot bound into DataFusion providers and UDFs.
//!
//! Built by [`crate::SqlEngine::bind`] or by the catalog host after a successful
//! load (`use_save` / `latest.*`). Providers hold `Arc<SessionBinding>` — they
//! do not re-read the filesystem.

use std::collections::BTreeSet;
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

    /// Display label for a good script name (defs labels, else prices row label).
    pub fn good_name(&self, good_name: &str) -> Option<String> {
        self.defs.labels.get(good_name).cloned().or_else(|| {
            self.prices
                .goods
                .iter()
                .find(|g| g.good_name == good_name)
                .and_then(|g| g.good_label.clone())
        })
    }

    /// Full [`alerts_with`] result, cached per bind and mitigation mode.
    ///
    /// Only the unfiltered full/lean cases are cached. Selective mitigation
    /// sets use [`Self::alerts_mitigating`] and are never stored as the fat cache.
    pub fn alerts(&self, with_mitigations: bool) -> Arc<AlertsResult> {
        cached_alerts(&self.alerts_cache, with_mitigations, || {
            alerts_with(
                self.world.as_ref(),
                self.defs.as_ref(),
                self.prices.as_ref(),
                AlertsOptions {
                    with_mitigations,
                    mitigation_ids: None,
                },
            )
        })
    }

    /// Detector pass with mitigations only for `ids` (not cached as full fat).
    pub fn alerts_mitigating(&self, ids: BTreeSet<String>) -> AlertsResult {
        if ids.is_empty() {
            return AlertsResult {
                alerts: Vec::new(),
                limitations: self.prices.limitations.clone(),
            };
        }
        alerts_with(
            self.world.as_ref(),
            self.defs.as_ref(),
            self.prices.as_ref(),
            AlertsOptions {
                with_mitigations: true,
                mitigation_ids: Some(ids),
            },
        )
    }

    /// Goods / electricity / transportation shortage alerts only.
    pub fn goods_shortage_alerts(&self, with_mitigations: bool) -> Arc<AlertsResult> {
        cached_alerts(&self.goods_alerts_cache, with_mitigations, || {
            goods_shortage_alerts(
                self.world.as_ref(),
                self.defs.as_ref(),
                self.prices.as_ref(),
                AlertsOptions {
                    with_mitigations,
                    mitigation_ids: None,
                },
            )
        })
    }

    /// Goods-shortage detectors with mitigations only for `ids`.
    pub fn goods_shortage_alerts_mitigating(&self, ids: BTreeSet<String>) -> AlertsResult {
        if ids.is_empty() {
            return AlertsResult {
                alerts: Vec::new(),
                limitations: self.prices.limitations.clone(),
            };
        }
        goods_shortage_alerts(
            self.world.as_ref(),
            self.defs.as_ref(),
            self.prices.as_ref(),
            AlertsOptions {
                with_mitigations: true,
                mitigation_ids: Some(ids),
            },
        )
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

/// Whether a DataFusion projection includes `col` (`None` = all columns).
pub fn projection_includes(projection: Option<&Vec<usize>>, col: usize) -> bool {
    match projection {
        None => true,
        Some(cols) => cols.contains(&col),
    }
}

/// Whether a DataFusion projection includes any of `json_cols`.
///
/// Prefer [`projection_includes`] for mitigations-only checks — evidence alone
/// must not force mitigation builders.
pub fn projection_needs_json(projection: Option<&Vec<usize>>, json_cols: &[usize]) -> bool {
    match projection {
        None => true,
        Some(cols) => cols.iter().any(|i| json_cols.contains(i)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_projection_does_not_need_mitigations() {
        // alerts_schema: evidence=8, mitigations=9
        assert!(projection_includes(Some(&vec![8]), 8));
        assert!(!projection_includes(Some(&vec![8]), 9));
        assert!(projection_includes(Some(&vec![9]), 9));
        assert!(projection_includes(None, 9));
        // Legacy helper still ORs columns — do not use it for mitigations gating.
        assert!(projection_needs_json(Some(&vec![8]), &[8, 9]));
    }
}
