//! Compact post-`use_save` campaign summary for MCP tool `campaign_brief`.
//!
//! Reads [`SessionBinding`] directly (no SQL round-trip): domestic goods
//! shortages, state×good hotspots, and a player-scoped alert-kind histogram
//! matching zero-arg `alerts()` filtering.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};
use vic3_prices::{AlertKind, World};
use vic3_sql::binding::{goods_shortage, SessionBinding};
use vic3_sql::ActiveSessionInfo;

const TOP_GOODS_LIMIT: usize = 10;
const HOTSPOT_LIMIT: usize = 15;

/// Build the `campaign_brief` JSON payload from an active bind.
pub(crate) fn campaign_brief_json(session: &ActiveSessionInfo, binding: &SessionBinding) -> Value {
    let player_tag = binding.world.player_tag.clone();
    let player_states = player_owned_state_ids(binding.world.as_ref());

    let region_by_id: BTreeMap<u32, String> = binding
        .prices
        .states
        .iter()
        .map(|s| {
            let name = s
                .label
                .clone()
                .or_else(|| s.region_name.clone())
                .unwrap_or_else(|| format!("state {}", s.id));
            (s.id, name)
        })
        .collect();

    let mut good_sums: BTreeMap<String, f64> = BTreeMap::new();
    let mut hotspots: Vec<(String, String, f64)> = Vec::new();

    for g in &binding.prices.state_goods {
        let owned = player_states
            .as_ref()
            .is_some_and(|ids| ids.contains(&g.state_id));
        if !owned {
            continue;
        }
        let shortage = goods_shortage(g.buy, g.sell);
        if shortage <= 0.0 {
            continue;
        }
        *good_sums.entry(g.name.clone()).or_insert(0.0) += shortage;
        let region = region_by_id
            .get(&g.state_id)
            .cloned()
            .unwrap_or_else(|| format!("state {}", g.state_id));
        hotspots.push((region, g.name.clone(), shortage));
    }

    let mut top_goods: Vec<(String, f64)> = good_sums.into_iter().collect();
    top_goods.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    top_goods.truncate(TOP_GOODS_LIMIT);

    hotspots.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    hotspots.truncate(HOTSPOT_LIMIT);

    let alert_kinds = player_alert_kind_histogram(binding, player_states.as_ref());

    json!({
        "session": {
            "name": session.name,
            "kind": session.kind,
            "in_game_date": session.in_game_date,
            "country": session.country,
        },
        "player_tag": player_tag,
        "top_goods": top_goods.iter().map(|(good, shortage)| json!({
            "good": good,
            "shortage": shortage,
        })).collect::<Vec<_>>(),
        "hotspots": hotspots.iter().map(|(state_name, good, shortage)| json!({
            "state_name": state_name,
            "good": good,
            "shortage": shortage,
        })).collect::<Vec<_>>(),
        "alert_kinds": alert_kinds,
    })
}

fn player_alert_kind_histogram(
    binding: &SessionBinding,
    player_states: Option<&BTreeSet<u32>>,
) -> BTreeMap<String, usize> {
    let result = binding.alerts(false);
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for alert in &result.alerts {
        let keep = match alert.state_id {
            None => true,
            Some(id) => player_states.is_some_and(|ids| ids.contains(&id)),
        };
        if !keep {
            continue;
        }
        *counts
            .entry(alert_kind_str(alert.kind).to_string())
            .or_insert(0) += 1;
    }
    counts
}

/// State ids owned by [`World::player_tag`] (strict; no first-country fallback).
///
/// Same filter as zero-arg SQL `alerts()`.
fn player_owned_state_ids(world: &World) -> Option<BTreeSet<u32>> {
    let tag = world.player_tag.as_deref()?;
    let country = world.country_by_tag(tag)?;
    let mut ids: BTreeSet<u32> = country.states.iter().copied().collect();
    for state in &world.states {
        if state.country == Some(country.id) {
            ids.insert(state.id);
        }
    }
    Some(ids)
}

fn alert_kind_str(kind: AlertKind) -> &'static str {
    match kind {
        AlertKind::ElectricityShortage => "electricity_shortage",
        AlertKind::TransportationShortage => "transportation_shortage",
        AlertKind::GoodsShortage => "goods_shortage",
        AlertKind::NeedsUnmet => "needs_unmet",
        AlertKind::LowMarketAccess => "low_market_access",
        AlertKind::UnfilledEducation => "unfilled_education",
        AlertKind::UnfilledPops => "unfilled_pops",
        AlertKind::Underemployed => "underemployed",
    }
}

#[cfg(test)]
mod tests {
    use super::alert_kind_str;
    use vic3_prices::AlertKind;

    #[test]
    fn alert_kind_strings_match_sql() {
        assert_eq!(alert_kind_str(AlertKind::GoodsShortage), "goods_shortage");
        assert_eq!(alert_kind_str(AlertKind::Underemployed), "underemployed");
    }
}
