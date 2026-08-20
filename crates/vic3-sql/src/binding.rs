//! In-memory analysis snapshot bound into DataFusion providers and UDFs.
//!
//! Built by [`crate::SqlEngine::bind`] or by the catalog host after a successful
//! load (`use_save` / `latest.*`). Providers hold `Arc<SessionBinding>` — they
//! do not re-read the filesystem.

use std::sync::Arc;

use vic3_defs::GameDefs;
use vic3_prices::{PricesResult, World};

/// In-memory analysis snapshot bound into DataFusion providers.
#[derive(Debug, Clone)]
pub struct SessionBinding {
    pub defs: Arc<GameDefs>,
    pub world: Arc<World>,
    pub prices: Arc<PricesResult>,
}

impl SessionBinding {
    /// Wrap owned analysis pieces for provider/UDF sharing.
    pub fn new(defs: GameDefs, world: World, prices: PricesResult) -> Self {
        Self {
            defs: Arc::new(defs),
            world: Arc::new(world),
            prices: Arc::new(prices),
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
}

/// Shortage volumes for goods tables (`docs/sql.md` open question #1 — locked).
///
/// `max(0, buy − sell)` — unmet demand after sell orders. Not Paradox’s
/// shortage flag; aligns with qualification `shortage = max(0, jobs − stock)`.
pub fn goods_shortage(buy: f64, sell: f64) -> f64 {
    (buy - sell).max(0.0)
}
