//! Shortage alerts and ranked mitigations.
//!
//! Detection uses an existing [`PricesResult`] plus the [`World`] it came from.
//! Heuristics are documented next to each detector; they are not a full Vic3
//! simulation.
//!
//! Profession adjacency when `common/pop_types` is not in defs:
//! `peasant → farmer/laborer → miner → machinist → engineer`, with academics
//! treated as university-adjacent.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vic3_defs::GameDefs;

use crate::result::{BuildingEconomics, ExtraLevelsDelta, ProductionMethodDelta, WorldDelta};
use crate::{
    apply_delta, market_access, price, GoodPrice, PricesResult, StateNeed, World, WorldBuilding,
    ORDER_EPS,
};

/// Market access at or below this ratio is a [`AlertKind::LowMarketAccess`].
const ACCESS_ALERT: f64 = 0.95;

/// Literacy below this (literate / demand size) is treated as far from university.
const LITERACY_UNIVERSITY: f64 = 0.5;

/// Alerts payload for wasm / CLI JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AlertsResult {
    pub alerts: Vec<Alert>,
    pub limitations: Vec<String>,
}

/// One shortage (or underemployment) expander in the Alerts pane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Alert {
    pub id: String,
    pub kind: AlertKind,
    /// `1` is urgent; [`AlertKind::Underemployed`] is `2`.
    pub severity: u8,
    pub title: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub building_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub good_id: Option<String>,
    pub evidence: Vec<Evidence>,
    pub mitigations: Vec<Mitigation>,
}

/// Detector kind. Snake-case in JSON.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    ElectricityShortage,
    TransportationShortage,
    GoodsShortage,
    NeedsUnmet,
    LowMarketAccess,
    UnfilledEducation,
    UnfilledPops,
    Underemployed,
}

/// Labelled observation shown under an expander.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Evidence {
    pub label: String,
    pub value: String,
}

/// Ranked advice. `apply_ready` is always false in this track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Mitigation {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub rank: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<MitigationAction>,
    pub apply_ready: bool,
    /// Estimated impact on the short good from current building IO or the
    /// good's `traded_quantity`. Not a market re-solve. Omitted outside
    /// shortage alerts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
}

/// Suggested follow-up. Internally tagged with `type`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MitigationAction {
    Build {
        building: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        state_id: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        extra_levels: Option<u32>,
    },
    Pm {
        building_id: u32,
        production_method: String,
        /// Full method list for Apply. Empty means “this one id only.”
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        methods: Vec<String>,
    },
    Subsidize {
        building_id: u32,
    },
    TradeAlloc {
        state_id: u32,
        good_id: String,
    },
    FeederJob {
        building: String,
        profession: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        state_id: Option<u32>,
    },
    SolGoods {
        good_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        state_id: Option<u32>,
    },
}

/// Diagnose shortages from a solved market. Does not mutate `world` or `prices`.
///
/// Needs-unmet uses a documented heuristic: a state need is unmet when any
/// basket good has local sell below demanded quantity, or local/package prices
/// sit at the `base * (1 + PRICE_RANGE)` ceiling.
pub fn alerts(world: &World, defs: &GameDefs, prices: &PricesResult) -> AlertsResult {
    let mut alerts = Vec::new();
    let mut extra_limitations = BTreeSet::new();

    collect_goods_alerts(world, defs, prices, &mut alerts, &mut extra_limitations);
    collect_needs_unmet(defs, prices, &mut alerts, &mut extra_limitations);
    collect_market_access(prices, world, &mut alerts);
    collect_education(world, prices, &mut alerts);
    collect_pop_and_underemployed(world, defs, prices, &mut alerts, &mut extra_limitations);

    let mut limitations = prices.limitations.clone();
    for line in extra_limitations {
        if !limitations.iter().any(|existing| existing == &line) {
            limitations.push(line);
        }
    }
    AlertsResult {
        alerts,
        limitations,
    }
}

fn collect_goods_alerts(
    world: &World,
    defs: &GameDefs,
    prices: &PricesResult,
    alerts: &mut Vec<Alert>,
    limitations: &mut BTreeSet<String>,
) {
    let mut seen = BTreeSet::new();
    let mut short_goods: BTreeMap<String, GoodsShortageHint> = BTreeMap::new();

    for row in &prices.goods {
        if good_is_short(row, defs) {
            short_goods.insert(row.id.clone(), GoodsShortageHint::from_row(row, None, None));
        }
    }
    for building in &prices.buildings {
        for good_id in &building.short_inputs {
            short_goods
                .entry(good_id.clone())
                .and_modify(|hint| {
                    hint.building_id = Some(building.id);
                    hint.state_id = building.state_id;
                })
                .or_insert_with(|| {
                    let row = prices.goods.iter().find(|good| good.id == *good_id);
                    GoodsShortageHint {
                        buy: row.map(|g| g.buy).unwrap_or(0.0),
                        sell: row.map(|g| g.sell).unwrap_or(0.0),
                        price: row.map(|g| g.price).unwrap_or(0.0),
                        base: row.map(|g| g.base).unwrap_or(0.0),
                        name: row.and_then(|g| g.name.clone()),
                        building_id: Some(building.id),
                        state_id: building.state_id,
                        from_short_inputs: true,
                    }
                });
        }
    }

    for (good_id, hint) in short_goods {
        let kind = goods_kind(&good_id);
        let key = (kind, good_id.clone());
        if !seen.insert(key) {
            continue;
        }
        let tradeable = is_tradeable(&good_id, defs);
        let display = hint.name.clone().unwrap_or_else(|| {
            defs.labels
                .get(&good_id)
                .cloned()
                .unwrap_or_else(|| good_id.clone())
        });
        let mut evidence = vec![
            Evidence {
                label: "Buy".into(),
                value: format_num(hint.buy),
            },
            Evidence {
                label: "Sell".into(),
                value: format_num(hint.sell),
            },
            Evidence {
                label: "Price / base".into(),
                value: format!("{} / {}", format_num(hint.price), format_num(hint.base)),
            },
        ];
        if hint.from_short_inputs {
            evidence.push(Evidence {
                label: "Short input".into(),
                value: good_id.clone(),
            });
        }
        if !tradeable {
            evidence.push(Evidence {
                label: "Trade".into(),
                value: "non-tradeable (local-only)".into(),
            });
        }
        let mitigations = goods_mitigations(
            &ShortageEffect {
                world,
                defs,
                prices,
                good_id: &good_id,
                buy: hint.buy,
                sell: hint.sell,
                base: hint.base,
            },
            hint.state_id,
            tradeable,
            limitations,
        );
        alerts.push(Alert {
            id: format!("{}:{good_id}", kind_id(kind)),
            kind,
            severity: 1,
            title: format!("{display} shortage"),
            summary: goods_summary(kind, tradeable, &hint),
            state_id: hint.state_id,
            building_id: hint.building_id,
            good_id: Some(good_id),
            evidence,
            mitigations,
        });
    }
}

struct GoodsShortageHint {
    buy: f64,
    sell: f64,
    price: f64,
    base: f64,
    name: Option<String>,
    building_id: Option<u32>,
    state_id: Option<u32>,
    from_short_inputs: bool,
}

impl GoodsShortageHint {
    fn from_row(row: &GoodPrice, building_id: Option<u32>, state_id: Option<u32>) -> Self {
        Self {
            buy: row.buy,
            sell: row.sell,
            price: row.price,
            base: row.base,
            name: row.name.clone(),
            building_id,
            state_id,
            from_short_inputs: false,
        }
    }
}

fn goods_summary(kind: AlertKind, tradeable: bool, hint: &GoodsShortageHint) -> String {
    let trade = if tradeable {
        "Trade-center import advice is included."
    } else {
        "This good is local-only; imports cannot help."
    };
    let kind_note = match kind {
        AlertKind::ElectricityShortage => "Electricity is short",
        AlertKind::TransportationShortage => "Transportation is short",
        _ => "A tradeable (or other) good is short",
    };
    format!(
        "{kind_note}: buy {} vs sell {}. {trade}",
        format_num(hint.buy),
        format_num(hint.sell)
    )
}

fn good_is_short(row: &GoodPrice, defs: &GameDefs) -> bool {
    if row.buy > row.sell + ORDER_EPS {
        return true;
    }
    let ceiling = row.base * (1.0 + defs.price_range.max(0.0));
    row.price + ORDER_EPS >= ceiling && (row.buy > ORDER_EPS || row.sell > ORDER_EPS)
}

fn goods_kind(good_id: &str) -> AlertKind {
    if id_has(good_id, "electricity") {
        AlertKind::ElectricityShortage
    } else if id_has(good_id, "transportation") {
        AlertKind::TransportationShortage
    } else {
        AlertKind::GoodsShortage
    }
}

/// Local-only when the id is electricity/transportation, or `traded_quantity` is 0.
fn is_tradeable(good_id: &str, defs: &GameDefs) -> bool {
    if id_has(good_id, "electricity") || id_has(good_id, "transportation") {
        return false;
    }
    defs.goods
        .get(good_id)
        .map(|good| good.traded_quantity > 0.0)
        .unwrap_or(true)
}

fn goods_mitigations(
    ctx: &ShortageEffect<'_>,
    state_id: Option<u32>,
    tradeable: bool,
    limitations: &mut BTreeSet<String>,
) -> Vec<Mitigation> {
    limitations.insert(
        "Shortage intervention effects hold pop demand fixed and revalue building IO after the suggested delta (a local price-formula step, not a full re-solve)."
            .into(),
    );
    let world = ctx.world;
    let defs = ctx.defs;
    let prices = ctx.prices;
    let good_id = ctx.good_id;
    let mut items = Vec::new();
    let alert_id = format!("goods:{good_id}");
    if !tradeable {
        items.push(with_effect(
            plain(
                format!("{alert_id}:local"),
                "Local-only good",
                format!(
                    "{good_id} is non-tradeable; skip import allocation and produce it in the state."
                ),
            ),
            format!("0 extra {good_id} from imports (local-only good)."),
        ));
        push_local_producer(&mut items, ctx, state_id, &alert_id);
        push_best_pm(&mut items, ctx, state_id, &alert_id);
        return rank(items);
    }

    let centers = trade_centers(prices, world, state_id);
    if centers.is_empty() {
        items.push(with_effect(
            action_mit(
                format!("{alert_id}:build-tc"),
                "Build a trade center",
                format!("No trade center in this state/market. Compare a new trade center against a local {good_id} producer."),
                MitigationAction::Build {
                    building: "building_trade_center".into(),
                    state_id,
                    extra_levels: Some(1),
                },
            ),
            ctx.extra_levels("building_trade_center", state_id, 1),
        ));
        push_local_producer(&mut items, ctx, state_id, &alert_id);
        push_best_pm(&mut items, ctx, state_id, &alert_id);
        return rank(items);
    }

    for center in &centers {
        let employed = center.staffing + ORDER_EPS >= center.level && center.level > 0.0;
        if !employed {
            if !center.short_inputs.is_empty() {
                items.push(with_effect(
                    plain(
                        format!("{alert_id}:tc-inputs-{}", center.id),
                        "Feed the trade center",
                        format!(
                            "Trade center {} is short on {}.",
                            center.id,
                            center.short_inputs.join(", ")
                        ),
                    ),
                    ctx.effect_from_staffing(center.id),
                ));
            }
            if center.profit < 0.0 {
                items.push(with_effect(
                    action_mit(
                        format!("{alert_id}:tc-subsidy-{}", center.id),
                        "Subsidize the trade center",
                        format!(
                            "Trade center {} is unprofitable and not fully employed.",
                            center.id
                        ),
                        MitigationAction::Subsidize {
                            building_id: center.id,
                        },
                    ),
                    ctx.effect_from_staffing(center.id),
                ));
            } else {
                items.push(with_effect(
                    plain(
                        format!("{alert_id}:tc-employ-{}", center.id),
                        "Staff the trade center",
                        format!(
                            "Trade center {} is at {} / {} levels. Raise employment before adding levels.",
                            center.id,
                            format_num(center.staffing),
                            format_num(center.level)
                        ),
                    ),
                    ctx.effect_from_staffing(center.id),
                ));
            }
        } else {
            items.push(with_effect(
                action_mit(
                    format!("{alert_id}:tc-levels-{}", center.id),
                    "Add trade-center levels",
                    format!("Trade center {} is fully employed; extra levels can raise throughput if this center moves {good_id}.", center.id),
                    MitigationAction::Build {
                        building: center.type_id.clone(),
                        state_id: center.state_id,
                        extra_levels: Some(1),
                    },
                ),
                ctx.extra_levels(&center.type_id, center.state_id.or(state_id), 1),
            ));
        }
        if let Some(state) = center.state_id.or(state_id) {
            if !state_imports(world, defs, state, good_id) {
                limitations.insert(
                    "Trade volumes are frozen in this model; import reallocation cannot be applied yet."
                        .into(),
                );
                items.push(with_effect(
                    action_mit(
                        format!("{alert_id}:reallocate-{state}"),
                        "Reallocate trade-center imports",
                        format!("State {state} is not importing {good_id}. Reallocation is advice only (frozen trade)."),
                        MitigationAction::TradeAlloc {
                            state_id: state,
                            good_id: good_id.into(),
                        },
                    ),
                    format!(
                        "0 extra {} in this model (trade volumes are frozen).",
                        ctx.good_id
                    ),
                ));
            }
        }
    }
    push_best_pm(&mut items, ctx, state_id, &alert_id);
    rank(items)
}

fn push_local_producer(
    items: &mut Vec<Mitigation>,
    ctx: &ShortageEffect<'_>,
    state_id: Option<u32>,
    alert_id: &str,
) {
    let producer = ctx.prices.buildings.iter().find(|building| {
        state_id.is_none_or(|sid| building.state_id == Some(sid))
            && building
                .outputs
                .iter()
                .any(|flow| flow.good_id == ctx.good_id)
    });
    let from_world = ctx.world.buildings.iter().find_map(|row| {
        if state_id.is_some_and(|sid| row.state != Some(sid)) {
            return None;
        }
        let idx = ctx.defs.index_of(ctx.good_id)?;
        let (inputs, outputs) = row.goods_io(ctx.defs);
        (outputs[idx] > ORDER_EPS || inputs[idx] > ORDER_EPS).then(|| row.building.clone())
    });
    let building = producer
        .map(|row| row.type_id.clone())
        .or(from_world)
        .unwrap_or_else(|| format!("building_{}_producer", ctx.good_id));
    items.push(with_effect(
        action_mit(
            format!("{alert_id}:local-producer"),
            "Expand a local producer",
            format!(
                "Raise local {} output as an alternative to trade.",
                ctx.good_id
            ),
            MitigationAction::Build {
                building: building.clone(),
                state_id,
                extra_levels: Some(1),
            },
        ),
        ctx.extra_levels(&building, state_id, 1),
    ));
}

fn push_best_pm(
    items: &mut Vec<Mitigation>,
    ctx: &ShortageEffect<'_>,
    state_id: Option<u32>,
    alert_id: &str,
) {
    let Some(pick) = best_pm_upgrade(ctx.world, ctx.defs, ctx.good_id, state_id) else {
        return;
    };
    let label = pm_label(ctx.defs, &pick.new_pm);
    let from = pick
        .from
        .iter()
        .map(|id| pm_label(ctx.defs, id))
        .collect::<Vec<_>>()
        .join(", ");
    let to = pick
        .to
        .iter()
        .map(|id| pm_label(ctx.defs, id))
        .collect::<Vec<_>>()
        .join(", ");
    let delta = WorldDelta {
        production_methods: vec![ProductionMethodDelta {
            building_id: pick.building_id,
            methods: pick.to.clone(),
        }],
        ..WorldDelta::default()
    };
    items.push(with_effect(
        action_mit(
            format!("{alert_id}:pm-{}", pick.building_id),
            format!("Switch to {label}"),
            format!(
                "On {} #{}, change production methods from [{from}] to [{to}].",
                pick.type_id, pick.building_id
            ),
            MitigationAction::Pm {
                building_id: pick.building_id,
                production_method: pick.new_pm,
                methods: pick.to,
            },
        ),
        ctx.effect_from_delta(&delta),
    ));
}

struct PmPick {
    building_id: u32,
    type_id: String,
    from: Vec<String>,
    to: Vec<String>,
    new_pm: String,
}

fn best_pm_upgrade(
    world: &World,
    defs: &GameDefs,
    good_id: &str,
    state_id: Option<u32>,
) -> Option<PmPick> {
    let idx = defs.index_of(good_id)?;
    let mut best: Option<PmPick> = None;
    let mut best_score = ORDER_EPS;
    for building in &world.buildings {
        if state_id.is_some_and(|sid| building.state != Some(sid)) {
            continue;
        }
        let current = &building.production_methods;
        if current.is_empty() {
            continue;
        }
        let candidates = type_pm_candidates(world, &building.building);
        if candidates.len() < 2 {
            continue;
        }
        let (in0, out0) = building.goods_io(defs);
        for slot in 0..current.len() {
            for candidate in &candidates {
                if current[slot] == *candidate {
                    continue;
                }
                let mut methods = current.clone();
                methods[slot] = candidate.clone();
                let trial = building.with_methods(methods.clone());
                let (in1, out1) = trial.goods_io(defs);
                let score = (out1[idx] - in1[idx]) - (out0[idx] - in0[idx]);
                if score > best_score {
                    best_score = score;
                    best = Some(PmPick {
                        building_id: building.id,
                        type_id: building.building.clone(),
                        from: current.clone(),
                        to: methods,
                        new_pm: candidate.clone(),
                    });
                }
            }
        }
    }
    best
}

fn type_pm_candidates(world: &World, type_id: &str) -> Vec<String> {
    let mut ids = BTreeSet::new();
    for building in &world.buildings {
        if building.building == type_id {
            ids.extend(building.production_methods.iter().cloned());
        }
    }
    ids.into_iter().collect()
}

fn pm_label(defs: &GameDefs, id: &str) -> String {
    defs.labels
        .get(id)
        .cloned()
        .unwrap_or_else(|| id.to_string())
}

struct ShortageEffect<'a> {
    world: &'a World,
    defs: &'a GameDefs,
    prices: &'a PricesResult,
    good_id: &'a str,
    buy: f64,
    sell: f64,
    base: f64,
}

impl ShortageEffect<'_> {
    fn gap(&self) -> f64 {
        (self.buy - self.sell).max(0.0)
    }

    fn extra_levels(&self, type_id: &str, state_id: Option<u32>, extra: u32) -> String {
        self.effect_from_delta(&extra_levels_on_type(self.world, type_id, state_id, extra))
    }

    fn effect_from_staffing(&self, building_id: u32) -> String {
        let mut next = self.world.clone();
        if let Some(building) = next.buildings.iter_mut().find(|b| b.id == building_id) {
            building.staffing = building.level.max(building.staffing);
        }
        self.effect_from_world(&next)
    }

    fn effect_from_delta(&self, delta: &WorldDelta) -> String {
        self.effect_from_world(&apply_delta(self.world, delta))
    }

    fn effect_from_world(&self, next: &World) -> String {
        let (buy0, sell0) = good_io_total(self.world, self.defs, self.good_id);
        let (buy1, sell1) = good_io_total(next, self.defs, self.good_id);
        let dbuy = buy1 - buy0;
        let dsell = sell1 - sell0;
        let new_buy = (self.buy + dbuy).max(0.0);
        let new_sell = (self.sell + dsell).max(0.0);
        let old_price = price(self.base, self.buy, self.sell, self.defs.price_range);
        let new_price = price(self.base, new_buy, new_sell, self.defs.price_range);
        format_local_effect(
            self.good_id,
            dsell,
            dbuy,
            old_price,
            new_price,
            self.gap(),
            (new_buy - new_sell).max(0.0),
        )
    }
}

fn extra_levels_on_type(
    world: &World,
    type_id: &str,
    state_id: Option<u32>,
    extra: u32,
) -> WorldDelta {
    let extra_levels = world
        .buildings
        .iter()
        .filter(|building| {
            building.building == type_id && state_id.is_none_or(|sid| building.state == Some(sid))
        })
        .map(|building| ExtraLevelsDelta {
            building: None,
            building_id: Some(building.id),
            extra_levels: extra,
        })
        .collect();
    WorldDelta {
        extra_levels,
        ..WorldDelta::default()
    }
}

fn good_io_total(world: &World, defs: &GameDefs, good_id: &str) -> (f64, f64) {
    let Some(idx) = defs.index_of(good_id) else {
        return (0.0, 0.0);
    };
    let mut buy = 0.0;
    let mut sell = 0.0;
    for building in &world.buildings {
        let (inputs, outputs) = building.goods_io(defs);
        buy += inputs[idx];
        sell += outputs[idx];
    }
    (buy, sell)
}

fn format_local_effect(
    good_id: &str,
    dsell: f64,
    dbuy: f64,
    old_price: f64,
    new_price: f64,
    old_gap: f64,
    new_gap: f64,
) -> String {
    let orders = if dsell.abs() <= ORDER_EPS && dbuy.abs() <= ORDER_EPS {
        format!("0 change to {good_id} building orders")
    } else {
        format!(
            "{} {good_id} sell, {} {good_id} buy",
            signed(dsell),
            signed(dbuy)
        )
    };
    let gap = if (new_gap - old_gap).abs() > ORDER_EPS {
        format!(
            "; gap {} → {}",
            format_num(old_gap),
            format_num(new_gap.max(0.0))
        )
    } else {
        String::new()
    };
    let price_bit = if (new_price - old_price).abs() > 1e-6 {
        format!(
            "; price {} → {}",
            format_num(old_price),
            format_num(new_price)
        )
    } else {
        "; price unchanged".into()
    };
    format!("{orders}{gap}{price_bit} (pop demand held).")
}

fn signed(value: f64) -> String {
    if value > ORDER_EPS {
        format!("+{}", format_num(value))
    } else if value < -ORDER_EPS {
        format!("-{}", format_num(-value))
    } else {
        "+0".into()
    }
}

fn collect_needs_unmet(
    defs: &GameDefs,
    prices: &PricesResult,
    alerts: &mut Vec<Alert>,
    limitations: &mut BTreeSet<String>,
) {
    // Heuristic: a state need is unmet when any basket good has local sell
    // below the demanded quantity, or local/package prices sit at the
    // `base * (1 + PRICE_RANGE)` ceiling.
    let ceiling_factor = 1.0 + defs.price_range.max(0.0);
    let mut by_state: BTreeMap<u32, Vec<&StateNeed>> = BTreeMap::new();
    for need in &prices.state_needs {
        if need_is_unmet(need, prices, ceiling_factor) {
            by_state.entry(need.state_id).or_default().push(need);
        }
    }
    if by_state.is_empty() {
        return;
    }
    limitations.insert(
        "Needs-unmet detection compares state need baskets to local sell, and flags high package prices near the PRICE_RANGE ceiling."
            .into(),
    );
    for (state_id, needs) in by_state {
        let mut evidence = Vec::new();
        let mut goods = Vec::new();
        for need in &needs {
            evidence.push(Evidence {
                label: need
                    .need_name
                    .clone()
                    .unwrap_or_else(|| need.need_id.clone()),
                value: format!("package {}", format_num(need.package_value)),
            });
            for flow in &need.goods {
                goods.push(flow.good_id.clone());
            }
        }
        let good_id = goods.first().cloned();
        let mitigations = need_mitigations(state_id, &goods);
        alerts.push(Alert {
            id: format!("needs_unmet:{state_id}"),
            kind: AlertKind::NeedsUnmet,
            severity: 1,
            title: format!("Unmet pop needs in state {state_id}"),
            summary: "Pop need baskets exceed local sell, or package prices are at the ceiling."
                .into(),
            state_id: Some(state_id),
            building_id: None,
            good_id,
            evidence,
            mitigations,
        });
    }
}

fn need_is_unmet(need: &StateNeed, prices: &PricesResult, ceiling_factor: f64) -> bool {
    if need.goods.is_empty() && need.package_value > 0.0 {
        return false;
    }
    for flow in &need.goods {
        if flow.quantity <= ORDER_EPS {
            continue;
        }
        let local = prices
            .state_goods
            .iter()
            .find(|row| row.state_id == need.state_id && row.good_id == flow.good_id);
        let Some(local) = local else {
            return true;
        };
        if local.sell + ORDER_EPS < flow.quantity {
            return true;
        }
        if local.price + ORDER_EPS >= local.base * ceiling_factor {
            return true;
        }
        let market = prices.goods.iter().find(|good| good.id == flow.good_id);
        if let Some(market) = market {
            if market.price + ORDER_EPS >= market.base * ceiling_factor {
                return true;
            }
        }
    }
    false
}

fn need_mitigations(state_id: u32, goods: &[String]) -> Vec<Mitigation> {
    let mut items = Vec::new();
    for good_id in goods.iter().take(3) {
        items.push(action_mit(
            format!("needs:{state_id}:sol:{good_id}"),
            format!("Cheapen {good_id}"),
            format!("Lower the local price of {good_id} (produce more or import) to cover the need basket."),
            MitigationAction::SolGoods {
                good_id: good_id.clone(),
                state_id: Some(state_id),
            },
        ));
        items.push(action_mit(
            format!("needs:{state_id}:import:{good_id}"),
            format!("Import {good_id} through a trade center"),
            format!("Trade-center imports of pop goods can fill the {good_id} basket."),
            MitigationAction::TradeAlloc {
                state_id,
                good_id: good_id.clone(),
            },
        ));
    }
    rank(items)
}

fn collect_market_access(prices: &PricesResult, world: &World, alerts: &mut Vec<Alert>) {
    let states: Vec<AccessState> = if prices.states.is_empty() {
        world
            .states
            .iter()
            .map(|state| AccessState {
                id: state.id,
                infrastructure: state.infrastructure,
                infrastructure_usage: state.infrastructure_usage,
            })
            .collect()
    } else {
        prices
            .states
            .iter()
            .map(|state| AccessState {
                id: state.id,
                infrastructure: state.infrastructure,
                infrastructure_usage: state.infrastructure_usage,
            })
            .collect()
    };
    for state in states {
        let access = market_access(state.infrastructure, state.infrastructure_usage);
        if access + ORDER_EPS >= ACCESS_ALERT {
            continue;
        }
        let infra = state.infrastructure.unwrap_or(0.0);
        let usage = state.infrastructure_usage.unwrap_or(0.0);
        alerts.push(Alert {
            id: format!("low_market_access:{}", state.id),
            kind: AlertKind::LowMarketAccess,
            severity: 1,
            title: format!("Low market access in state {}", state.id),
            summary: format!(
                "Infrastructure {} / usage {} is {:.0}% (threshold {:.0}%).",
                format_num(infra),
                format_num(usage),
                access * 100.0,
                ACCESS_ALERT * 100.0
            ),
            state_id: Some(state.id),
            building_id: None,
            good_id: None,
            evidence: vec![
                Evidence {
                    label: "Infrastructure".into(),
                    value: format_num(infra),
                },
                Evidence {
                    label: "Usage".into(),
                    value: format_num(usage),
                },
                Evidence {
                    label: "Access".into(),
                    value: format!("{:.2}", access),
                },
            ],
            mitigations: rank(vec![action_mit(
                format!("access:{}:rail", state.id),
                "Add infrastructure",
                "Build railways or urban infrastructure so usage no longer exceeds capacity.",
                MitigationAction::Build {
                    building: "building_railway".into(),
                    state_id: Some(state.id),
                    extra_levels: Some(1),
                },
            )]),
        });
    }
}

struct AccessState {
    id: u32,
    infrastructure: Option<f64>,
    infrastructure_usage: Option<f64>,
}

fn collect_education(world: &World, prices: &PricesResult, alerts: &mut Vec<Alert>) {
    for row in &prices.state_qualifications {
        if row.shortage <= ORDER_EPS {
            continue;
        }
        let target = row.profession_id.as_str();
        let mix = state_mix(prices, row.state_id);
        let mut items = qualification_ladder(prices, row.state_id, target, &mix);
        if has_unstaffed_university(prices, world, row.state_id) {
            items.retain(|item| !is_university_build(item));
            items.push(plain(
                format!("edu:{}:staff-uni", row.state_id),
                "Staff the existing university",
                "An unstaffed university is already in this state; do not queue another campus.",
            ));
        }
        alerts.push(Alert {
            id: format!("unfilled_education:{}:{target}", row.state_id),
            kind: AlertKind::UnfilledEducation,
            severity: 1,
            title: format!("{} qualification shortage", display_prof(row)),
            summary: format!(
                "State {} is short {} {} (jobs {} vs employable/qualified {}).",
                row.state_id,
                format_num(row.shortage),
                display_prof(row),
                format_num(row.jobs),
                format_num(row.employable.unwrap_or(row.qualified))
            ),
            state_id: Some(row.state_id),
            building_id: None,
            good_id: None,
            evidence: vec![
                Evidence {
                    label: "Profession".into(),
                    value: display_prof(row),
                },
                Evidence {
                    label: "Shortage".into(),
                    value: format_num(row.shortage),
                },
                Evidence {
                    label: "Literacy".into(),
                    value: format!("{:.0}%", mix.literacy * 100.0),
                },
                Evidence {
                    label: "Profession mix".into(),
                    value: mix.summary(),
                },
            ],
            mitigations: rank(items),
        });
    }
}

fn collect_pop_and_underemployed(
    world: &World,
    defs: &GameDefs,
    prices: &PricesResult,
    alerts: &mut Vec<Alert>,
    limitations: &mut BTreeSet<String>,
) {
    let buildings = if prices.buildings.is_empty() {
        world
            .buildings
            .iter()
            .map(economics_from_world)
            .collect::<Vec<_>>()
    } else {
        prices.buildings.clone()
    };
    for building in &buildings {
        if building.level <= ORDER_EPS || building.staffing + ORDER_EPS >= building.level {
            continue;
        }
        let ratio = building.staffing / building.level;
        let qual_short = has_employee_qual_shortage(prices, building);
        if !qual_short {
            limitations.insert(
                "Migration is frozen in this model; in-migration is a diagnostic label only."
                    .into(),
            );
            let mix = building
                .state_id
                .map(|id| state_mix(prices, id))
                .unwrap_or_default();
            alerts.push(Alert {
                id: format!("unfilled_pops:{}", building.id),
                kind: AlertKind::UnfilledPops,
                severity: 1,
                title: format!("Unfilled pops at {}", building.type_id),
                summary: format!(
                    "{} is staffed {} / {} with no qualification shortage for its employees.",
                    building.type_id,
                    format_num(building.staffing),
                    format_num(building.level)
                ),
                state_id: building.state_id,
                building_id: Some(building.id),
                good_id: None,
                evidence: pop_evidence(building, &mix, false),
                mitigations: rank(pop_shortage_mitigations(prices, defs, building, &mix)),
            });
        }
        alerts.push(Alert {
            id: format!("underemployed:{}", building.id),
            kind: AlertKind::Underemployed,
            severity: 2,
            title: format!("Underemployed {}", building.type_id),
            summary: format!(
                "Staffing/level is {:.0}% on {}.",
                ratio * 100.0,
                building.type_id
            ),
            state_id: building.state_id,
            building_id: Some(building.id),
            good_id: None,
            evidence: pop_evidence(building, &state_mix_opt(prices, building.state_id), qual_short),
            mitigations: rank(if qual_short {
                vec![plain(
                    format!("under:{}:qual", building.id),
                    "Fill qualifications first",
                    "Understaffing here tracks a qualification shortage; feeder jobs outrank extra levels.",
                )]
                .into_iter()
                .chain(qualification_ladder(
                    prices,
                    building.state_id.unwrap_or(0),
                    building
                        .employees
                        .first()
                        .map(|e| e.profession_id.as_str())
                        .unwrap_or("machinists"),
                    &state_mix_opt(prices, building.state_id),
                ))
                .collect()
            } else {
                pop_shortage_mitigations(prices, defs, building, &state_mix_opt(prices, building.state_id))
            }),
        });
    }
}

fn pop_evidence(building: &BuildingEconomics, mix: &StateMix, qual_short: bool) -> Vec<Evidence> {
    vec![
        Evidence {
            label: "Staffing / level".into(),
            value: format!(
                "{} / {}",
                format_num(building.staffing),
                format_num(building.level)
            ),
        },
        Evidence {
            label: "Qualification shortage".into(),
            value: if qual_short {
                "yes".into()
            } else {
                "no".into()
            },
        },
        Evidence {
            label: "Peasants".into(),
            value: format_num(mix.peasants),
        },
        Evidence {
            label: "In-migration".into(),
            value: "frozen in this model".into(),
        },
    ]
}

fn pop_shortage_mitigations(
    prices: &PricesResult,
    defs: &GameDefs,
    building: &BuildingEconomics,
    mix: &StateMix,
) -> Vec<Mitigation> {
    let mut items = Vec::new();
    let state_id = building.state_id;
    let pop_goods = expensive_pop_goods(prices, defs, state_id);
    for good_id in pop_goods.iter().take(2) {
        items.push(action_mit(
            format!("pops:{}:sol:{good_id}", building.id),
            format!("Raise SoL via cheaper {good_id}"),
            "Cheaper need goods raise standard of living before extra unstaffed levels help.",
            MitigationAction::SolGoods {
                good_id: good_id.clone(),
                state_id,
            },
        ));
        if is_tradeable(good_id, defs) {
            if let Some(sid) = state_id {
                items.push(action_mit(
                    format!("pops:{}:import:{good_id}", building.id),
                    format!("Import {good_id} through a trade center"),
                    "Trade-center imports of pop goods support in-migration once apply exists.",
                    MitigationAction::TradeAlloc {
                        state_id: sid,
                        good_id: good_id.clone(),
                    },
                ));
            }
        }
    }
    if mix.peasants > 0.0 || mix.dominant_rung() == Some(0) {
        let farm = building_like(prices, state_id, "farm").unwrap_or("building_rye_farm");
        items.push(action_mit(
            format!("pops:{}:peasants", building.id),
            "Employ peasants first",
            "Hire peasants into farms (then mines) rather than adding unstaffed high-tech levels.",
            MitigationAction::FeederJob {
                building: farm.into(),
                profession: "farmers".into(),
                state_id,
            },
        ));
    }
    items.push(plain(
        format!("pops:{}:migration", building.id),
        "In-migration is frozen",
        "This model does not simulate migration; extra unstaffed levels will not attract pops here.",
    ));
    items.push(action_mit(
        format!("pops:{}:levels", building.id),
        "Extra levels (last resort)",
        "More unstaffed levels do not fix a pop shortage. Ranked last on purpose.",
        MitigationAction::Build {
            building: building.type_id.clone(),
            state_id,
            extra_levels: Some(1),
        },
    ));
    items
}

fn expensive_pop_goods(
    prices: &PricesResult,
    defs: &GameDefs,
    state_id: Option<u32>,
) -> Vec<String> {
    let ceiling = 1.0 + defs.price_range.max(0.0);
    let mut scored = Vec::new();
    for need in &prices.state_needs {
        if state_id.is_some_and(|id| need.state_id != id) {
            continue;
        }
        for flow in &need.goods {
            let price = prices
                .goods
                .iter()
                .find(|good| good.id == flow.good_id)
                .map(|good| {
                    if good.base > 0.0 {
                        good.price / good.base
                    } else {
                        0.0
                    }
                })
                .unwrap_or(0.0);
            scored.push((flow.good_id.clone(), price));
        }
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.dedup_by(|a, b| a.0 == b.0);
    if scored.is_empty() {
        prices
            .goods
            .iter()
            .filter(|good| good.price + ORDER_EPS >= good.base * ceiling)
            .map(|good| good.id.clone())
            .collect()
    } else {
        scored.into_iter().map(|(id, _)| id).collect()
    }
}

fn qualification_ladder(
    prices: &PricesResult,
    state_id: u32,
    target: &str,
    mix: &StateMix,
) -> Vec<Mitigation> {
    let target_rung = profession_rung(target).unwrap_or(3);
    let current = mix.dominant_rung().unwrap_or(0);
    let far_off = target_rung.saturating_sub(current) >= 2 || mix.literacy < LITERACY_UNIVERSITY;
    let mut items = Vec::new();
    let farm = building_like(prices, Some(state_id), "farm").unwrap_or("building_rye_farm");
    let mine = building_like(prices, Some(state_id), "mine").unwrap_or("building_coal_mine");
    let shop = building_like(prices, Some(state_id), "workshop")
        .or_else(|| building_like(prices, Some(state_id), "manufactur"))
        .unwrap_or("building_tooling_workshop");

    if far_off || current <= 1 {
        items.push(action_mit(
            format!("ladder:{state_id}:farm"),
            "Build farms (qualification ladder)",
            "Peasants qualify toward farmers/laborers on farms before universities help machinists or engineers.",
            MitigationAction::FeederJob {
                building: farm.into(),
                profession: "farmers".into(),
                state_id: Some(state_id),
            },
        ));
        items.push(action_mit(
            format!("ladder:{state_id}:mine"),
            "Build mines (qualification ladder)",
            "Laborers on mines are the next step toward machinists.",
            MitigationAction::FeederJob {
                building: mine.into(),
                profession: "miners".into(),
                state_id: Some(state_id),
            },
        ));
        if target_rung >= 3 {
            items.push(action_mit(
                format!("ladder:{state_id}:workshop"),
                "Build workshops (qualification ladder)",
                "Workshops hire machinists once mines have filled the lower rungs.",
                MitigationAction::FeederJob {
                    building: shop.into(),
                    profession: "machinists".into(),
                    state_id: Some(state_id),
                },
            ));
        }
    }
    let uni_ok = !far_off
        && mix.literacy >= LITERACY_UNIVERSITY
        && (current + 1 >= target_rung || id_has(target, "academic"));
    let uni = building_like(prices, Some(state_id), "university").unwrap_or("building_university");
    if uni_ok || id_has(target, "academic") {
        items.push(action_mit(
            format!("ladder:{state_id}:university"),
            "University (in range)",
            "Literacy and nearby professions are close enough that a university can help.",
            MitigationAction::Build {
                building: uni.into(),
                state_id: Some(state_id),
                extra_levels: Some(1),
            },
        ));
    } else {
        items.push(action_mit(
            format!("ladder:{state_id}:university"),
            "University (after feeders)",
            "Universities come after farms and mines when the pop mix is still peasants.",
            MitigationAction::Build {
                building: uni.into(),
                state_id: Some(state_id),
                extra_levels: Some(1),
            },
        ));
    }
    items
}

fn has_employee_qual_shortage(prices: &PricesResult, building: &BuildingEconomics) -> bool {
    let Some(state_id) = building.state_id else {
        return false;
    };
    if building.employees.is_empty() {
        return false;
    }
    building.employees.iter().any(|employee| {
        prices.state_qualifications.iter().any(|row| {
            row.state_id == state_id
                && row.profession_id == employee.profession_id
                && row.shortage > ORDER_EPS
        })
    })
}

fn has_unstaffed_university(prices: &PricesResult, world: &World, state_id: u32) -> bool {
    prices.buildings.iter().any(|building| {
        building.state_id == Some(state_id)
            && id_has(&building.type_id, "university")
            && building.staffing + ORDER_EPS < building.level
    }) || world.buildings.iter().any(|building| {
        building.state == Some(state_id)
            && id_has(&building.building, "university")
            && building.staffing + ORDER_EPS < building.level
    })
}

fn is_university_build(item: &Mitigation) -> bool {
    match &item.action {
        Some(MitigationAction::Build { building, .. }) => id_has(building, "university"),
        _ => false,
    }
}

fn trade_centers<'a>(
    prices: &'a PricesResult,
    world: &'a World,
    state_id: Option<u32>,
) -> Vec<&'a BuildingEconomics> {
    let market =
        state_id.and_then(|id| state_market(prices, id).or_else(|| world_market(world, id)));
    let from_prices: Vec<&BuildingEconomics> = prices
        .buildings
        .iter()
        .filter(|building| {
            id_has(&building.type_id, "trade_center")
                && same_area(prices, building.state_id, state_id, market)
        })
        .collect();
    if !from_prices.is_empty() {
        return from_prices;
    }
    Vec::new()
}

fn same_area(
    prices: &PricesResult,
    building_state: Option<u32>,
    want_state: Option<u32>,
    market: Option<u32>,
) -> bool {
    match (building_state, want_state) {
        (Some(a), Some(b)) if a == b => true,
        (Some(a), Some(_)) => market.is_some() && state_market(prices, a) == market,
        (Some(_), None) => true,
        (None, _) => true,
    }
}

fn state_market(prices: &PricesResult, state_id: u32) -> Option<u32> {
    prices
        .states
        .iter()
        .find(|state| state.id == state_id)
        .and_then(|state| state.market_id)
}

fn world_market(world: &World, state_id: u32) -> Option<u32> {
    world
        .states
        .iter()
        .find(|state| state.id == state_id)
        .and_then(|state| state.market)
}

fn state_imports(world: &World, defs: &GameDefs, state_id: u32, good_id: &str) -> bool {
    let Some(idx) = defs.index_of(good_id) else {
        return false;
    };
    world
        .state_trade
        .iter()
        .any(|trade| trade.state == state_id && trade.good == idx && trade.quantity > ORDER_EPS)
}

fn building_like<'a>(
    prices: &'a PricesResult,
    state_id: Option<u32>,
    needle: &str,
) -> Option<&'a str> {
    prices.buildings.iter().find_map(|building| {
        if state_id.is_some_and(|id| building.state_id != Some(id)) {
            return None;
        }
        id_has(&building.type_id, needle).then_some(building.type_id.as_str())
    })
}

fn economics_from_world(building: &WorldBuilding) -> BuildingEconomics {
    BuildingEconomics {
        id: building.id,
        state_id: building.state,
        type_id: building.building.clone(),
        level: building.level,
        staffing: building.staffing,
        production_method_ids: building.production_methods.clone(),
        inputs: Vec::new(),
        outputs: Vec::new(),
        revenue: 0.0,
        cost: 0.0,
        profit: 0.0,
        short_inputs: Vec::new(),
        employees: Vec::new(),
    }
}

#[derive(Default)]
struct StateMix {
    peasants: f64,
    farmers: f64,
    miners: f64,
    machinists: f64,
    engineers: f64,
    academics: f64,
    literacy: f64,
}

impl StateMix {
    fn dominant_rung(&self) -> Option<u8> {
        let ranks = [
            (0_u8, self.peasants),
            (1, self.farmers),
            (2, self.miners),
            (3, self.machinists),
            (4, self.engineers),
            (5, self.academics),
        ];
        ranks
            .into_iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .filter(|(_, n)| *n > 0.0)
            .map(|(rung, _)| rung)
    }

    fn summary(&self) -> String {
        format!(
            "peasants {} / farmers {} / miners {} / machinists {}",
            format_num(self.peasants),
            format_num(self.farmers),
            format_num(self.miners),
            format_num(self.machinists)
        )
    }
}

fn state_mix_opt(prices: &PricesResult, state_id: Option<u32>) -> StateMix {
    state_id.map(|id| state_mix(prices, id)).unwrap_or_default()
}

fn state_mix(prices: &PricesResult, state_id: u32) -> StateMix {
    let mut mix = StateMix::default();
    let mut literate = 0.0;
    let mut size = 0.0;
    for pop in prices.state_pops.iter() {
        if pop.state_id != state_id {
            continue;
        }
        let n = pop.workforce.or(pop.demand_size).unwrap_or(0.0);
        size += pop.demand_size.unwrap_or(n);
        literate += pop.literate.unwrap_or(0.0);
        let Some(prof) = pop.profession_id.as_deref() else {
            continue;
        };
        match profession_rung(prof) {
            Some(0) => mix.peasants += n,
            Some(1) => mix.farmers += n,
            Some(2) => mix.miners += n,
            Some(3) => mix.machinists += n,
            Some(4) => mix.engineers += n,
            Some(5) => mix.academics += n,
            _ => {}
        }
    }
    mix.literacy = if size > 0.0 { literate / size } else { 0.0 };
    mix
}

/// Peasant → farmer/laborer → miner → machinist → engineer; academics at university.
fn profession_rung(id: &str) -> Option<u8> {
    let id = id.to_ascii_lowercase();
    if id.contains("peasant") {
        Some(0)
    } else if id.contains("farmer") || id.contains("laborer") || id.contains("labourer") {
        Some(1)
    } else if id.contains("miner") {
        Some(2)
    } else if id.contains("machinist") {
        Some(3)
    } else if id.contains("engineer") {
        Some(4)
    } else if id.contains("academic") {
        Some(5)
    } else {
        None
    }
}

fn display_prof(row: &crate::StateQualification) -> String {
    row.profession_name
        .clone()
        .unwrap_or_else(|| row.profession_id.clone())
}

fn id_has(id: &str, needle: &str) -> bool {
    id.to_ascii_lowercase().contains(needle)
}

fn kind_id(kind: AlertKind) -> &'static str {
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

fn format_num(value: f64) -> String {
    if (value - value.round()).abs() < 1e-6 {
        format!("{:.0}", value.round())
    } else {
        format!("{value:.2}")
    }
}

fn plain(id: String, title: impl Into<String>, detail: impl Into<String>) -> Mitigation {
    Mitigation {
        id,
        title: title.into(),
        detail: detail.into(),
        rank: 0,
        action: None,
        apply_ready: false,
        effect: None,
    }
}

fn action_mit(
    id: String,
    title: impl Into<String>,
    detail: impl Into<String>,
    action: MitigationAction,
) -> Mitigation {
    Mitigation {
        id,
        title: title.into(),
        detail: detail.into(),
        rank: 0,
        action: Some(action),
        apply_ready: false,
        effect: None,
    }
}

fn with_effect(mut item: Mitigation, effect: impl Into<String>) -> Mitigation {
    item.effect = Some(effect.into());
    item
}

fn rank(mut items: Vec<Mitigation>) -> Vec<Mitigation> {
    for (index, item) in items.iter_mut().enumerate() {
        item.rank = u32::try_from(index + 1).unwrap_or(u32::MAX);
        item.apply_ready = false;
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BuildingEconomics, GoodFlow, GoodPrice, MarketInputs, ProfessionCount, SolveStatus,
        StateGood, StateInfo, StateNeed, StatePop, StateQualification, WorldBuilding, WorldState,
        WorldStateTrade,
    };
    use vic3_defs::{GameDefs, Good, GoodIdx, ProductionMethod};

    fn defs() -> GameDefs {
        let mut defs = GameDefs {
            price_range: 0.75,
            goods_order: vec![
                "grain".into(),
                "electricity".into(),
                "transportation".into(),
                "clothes".into(),
                "iron".into(),
            ],
            ..GameDefs::default()
        };
        for (id, base, traded) in [
            ("grain", 20.0, 12.0),
            ("electricity", 30.0, 0.0),
            ("transportation", 30.0, 0.0),
            ("clothes", 30.0, 10.0),
            ("iron", 40.0, 8.0),
        ] {
            defs.goods.insert(
                id.into(),
                Good {
                    id: id.into(),
                    base_price: base,
                    traded_quantity: traded,
                    texture: None,
                },
            );
        }
        let grain = GoodIdx::from_usize(0);
        defs.production_methods.insert(
            "pm_simple_farming".into(),
            ProductionMethod {
                id: "pm_simple_farming".into(),
                inputs: Vec::new(),
                outputs: vec![(grain, 10.0)],
            },
        );
        defs.production_methods.insert(
            "pm_soil_enriching_farming".into(),
            ProductionMethod {
                id: "pm_soil_enriching_farming".into(),
                inputs: Vec::new(),
                outputs: vec![(grain, 25.0)],
            },
        );
        defs.labels.insert(
            "pm_soil_enriching_farming".into(),
            "Soil Enriching Farming".into(),
        );
        defs
    }

    fn good(id: &str, base: f64, price: f64, buy: f64, sell: f64) -> GoodPrice {
        GoodPrice {
            id: id.into(),
            name: Some(id.into()),
            base,
            price,
            buy,
            sell,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn econ(
        id: u32,
        state: u32,
        type_id: &str,
        level: f64,
        staffing: f64,
        employees: &[(&str, f64)],
        short_inputs: &[&str],
        profit: f64,
    ) -> BuildingEconomics {
        BuildingEconomics {
            id,
            state_id: Some(state),
            type_id: type_id.into(),
            level,
            staffing,
            production_method_ids: vec!["pm_default".into()],
            inputs: Vec::new(),
            outputs: Vec::new(),
            revenue: profit.max(0.0) + 10.0,
            cost: 10.0,
            profit,
            short_inputs: short_inputs.iter().map(|s| (*s).to_string()).collect(),
            employees: employees
                .iter()
                .map(|(prof, count)| ProfessionCount {
                    profession_id: (*prof).into(),
                    profession_name: None,
                    count: *count,
                })
                .collect(),
        }
    }

    fn world_building(id: u32, state: u32, kind: &str, level: f64, staffing: f64) -> WorldBuilding {
        world_building_pm(id, state, kind, level, staffing, &["pm_default"])
    }

    fn world_building_pm(
        id: u32,
        state: u32,
        kind: &str,
        level: f64,
        staffing: f64,
        methods: &[&str],
    ) -> WorldBuilding {
        WorldBuilding {
            id,
            state: Some(state),
            building: kind.into(),
            level,
            staffing,
            production_methods: methods.iter().map(|id| (*id).to_string()).collect(),
            saved_inputs: Vec::new(),
            saved_outputs: Vec::new(),
        }
    }

    fn fixture() -> (World, GameDefs, PricesResult) {
        let defs = defs();
        let grain = defs.index_of("grain").expect("grain");
        let world = World {
            states: vec![WorldState {
                id: 1,
                market: Some(1),
                infrastructure: Some(40.0),
                infrastructure_usage: Some(100.0),
                ..WorldState::default()
            }],
            buildings: vec![
                world_building(1, 1, "building_trade_center", 2.0, 2.0),
                world_building(2, 1, "building_university", 1.0, 0.0),
                world_building_pm(3, 1, "building_rye_farm", 3.0, 1.0, &["pm_simple_farming"]),
                world_building(4, 1, "building_tooling_workshop", 2.0, 0.5),
                world_building_pm(
                    5,
                    1,
                    "building_rye_farm",
                    1.0,
                    1.0,
                    &["pm_soil_enriching_farming"],
                ),
            ],
            state_trade: vec![WorldStateTrade {
                state: 1,
                good: grain,
                quantity: -4.0,
            }],
            ..World::default()
        };
        let prices = PricesResult {
            scope: "whole_save_synthetic".into(),
            goods: vec![
                good("grain", 20.0, 35.0, 80.0, 10.0),
                good("electricity", 30.0, 52.5, 40.0, 5.0),
                good("transportation", 30.0, 52.5, 25.0, 2.0),
                good("clothes", 30.0, 52.5, 12.0, 1.0),
                good("iron", 40.0, 40.0, 5.0, 5.0),
            ],
            countries: Vec::new(),
            states: vec![StateInfo {
                id: 1,
                region_id: Some("STATE_TEST".into()),
                region_name: Some("Test".into()),
                country_id: Some(1),
                market_id: Some(1),
                arable_land: None,
                infrastructure: Some(40.0),
                infrastructure_usage: Some(100.0),
            }],
            state_goods: vec![StateGood {
                state_id: 1,
                good_id: "clothes".into(),
                buy: 12.0,
                sell: 0.0,
                price: 52.5,
                market_price: 52.5,
                state_price: 52.5,
                market_access: 0.4,
                effective_mapi: 0.3,
                base: 30.0,
            }],
            buildings: vec![
                econ(1, 1, "building_trade_center", 2.0, 2.0, &[], &[], 15.0),
                econ(
                    2,
                    1,
                    "building_university",
                    1.0,
                    0.0,
                    &[("academics", 0.0)],
                    &[],
                    -1.0,
                ),
                {
                    let mut farm = econ(
                        3,
                        1,
                        "building_rye_farm",
                        3.0,
                        1.0,
                        &[("farmers", 4000.0)],
                        &[],
                        5.0,
                    );
                    farm.outputs.push(GoodFlow {
                        good_id: "grain".into(),
                        quantity: 30.0,
                        value: 600.0,
                    });
                    farm
                },
                econ(
                    4,
                    1,
                    "building_tooling_workshop",
                    2.0,
                    0.5,
                    &[("machinists", 800.0)],
                    &["electricity"],
                    2.0,
                ),
            ],
            building_types: Vec::new(),
            building_groups: Vec::new(),
            state_pops: vec![
                StatePop {
                    state_id: 1,
                    id: Some(1),
                    profession_id: Some("peasants".into()),
                    profession_name: Some("Peasants".into()),
                    demand_size: Some(20_000.0),
                    workforce: Some(12_000.0),
                    dependents: Some(8_000.0),
                    wealth: Some(8),
                    culture_id: None,
                    culture_name: None,
                    literate: Some(1_000.0),
                    workplace_id: None,
                    qualifications: Vec::new(),
                    needs: Vec::new(),
                },
                StatePop {
                    state_id: 1,
                    id: Some(2),
                    profession_id: Some("farmers".into()),
                    profession_name: Some("Farmers".into()),
                    demand_size: Some(4_000.0),
                    workforce: Some(4_000.0),
                    dependents: Some(0.0),
                    wealth: Some(10),
                    culture_id: None,
                    culture_name: None,
                    literate: Some(400.0),
                    workplace_id: Some(3),
                    qualifications: Vec::new(),
                    needs: Vec::new(),
                },
            ]
            .into(),
            state_qualifications: vec![StateQualification {
                state_id: 1,
                profession_id: "machinists".into(),
                profession_name: Some("Machinists".into()),
                qualified: 800.0,
                employable: Some(800.0),
                employed: 800.0,
                jobs: 5_000.0,
                shortage: 4_200.0,
                monthly_change: None,
            }],
            state_needs: vec![StateNeed {
                state_id: 1,
                need_id: "popneed_clothing".into(),
                need_name: Some("Clothing".into()),
                package_value: 80.0,
                goods: vec![GoodFlow {
                    good_id: "clothes".into(),
                    quantity: 12.0,
                    value: 80.0,
                }],
            }],
            inputs: MarketInputs::default(),
            residual: 0.0,
            status: SolveStatus::Converged,
            limitations: vec![
                "Employment, wages, and trade volumes are frozen except explicit what-if deltas."
                    .into(),
            ],
            relative: Vec::new(),
        };
        (world, defs, prices)
    }

    fn kinds(result: &AlertsResult) -> BTreeSet<AlertKind> {
        result.alerts.iter().map(|alert| alert.kind).collect()
    }

    #[test]
    fn each_kind_fires_on_fixture() {
        let (world, defs, prices) = fixture();
        let result = alerts(&world, &defs, &prices);
        for kind in [
            AlertKind::ElectricityShortage,
            AlertKind::TransportationShortage,
            AlertKind::GoodsShortage,
            AlertKind::NeedsUnmet,
            AlertKind::LowMarketAccess,
            AlertKind::UnfilledEducation,
            AlertKind::UnfilledPops,
            AlertKind::Underemployed,
        ] {
            assert!(
                kinds(&result).contains(&kind),
                "missing {kind:?} in {:?}",
                kinds(&result)
            );
        }
        assert!(result
            .alerts
            .iter()
            .all(|alert| { alert.mitigations.iter().all(|mit| !mit.apply_ready) }));
    }

    #[test]
    fn tradeable_shortage_includes_trade_center_mitigation() {
        let (world, defs, prices) = fixture();
        let result = alerts(&world, &defs, &prices);
        let grain = result
            .alerts
            .iter()
            .find(|alert| {
                alert.kind == AlertKind::GoodsShortage && alert.good_id.as_deref() == Some("grain")
            })
            .expect("grain shortage");
        assert!(
            grain.mitigations.iter().any(|mit| {
                mit.title.to_ascii_lowercase().contains("trade")
                    || matches!(
                        mit.action,
                        Some(MitigationAction::TradeAlloc { .. })
                            | Some(MitigationAction::Build { .. })
                            | Some(MitigationAction::Pm { .. })
                            | Some(MitigationAction::Subsidize { .. })
                    )
            }),
            "expected trade-center mitigation, got {:?}",
            grain
                .mitigations
                .iter()
                .map(|m| &m.title)
                .collect::<Vec<_>>()
        );
        assert!(result
            .limitations
            .iter()
            .any(|line| line.to_ascii_lowercase().contains("frozen")
                && line.to_ascii_lowercase().contains("trade")));
        let power = result
            .alerts
            .iter()
            .find(|alert| alert.kind == AlertKind::ElectricityShortage)
            .expect("electricity");
        assert!(power.mitigations.iter().any(|mit| mit
            .detail
            .to_ascii_lowercase()
            .contains("non-tradeable")
            || mit.title.to_ascii_lowercase().contains("local-only")));
        assert!(!power
            .mitigations
            .iter()
            .any(|mit| matches!(mit.action, Some(MitigationAction::TradeAlloc { .. }))));
    }

    #[test]
    fn peasant_state_ranks_farm_and_mine_above_university() {
        let (world, defs, prices) = fixture();
        let result = alerts(&world, &defs, &prices);
        let edu = result
            .alerts
            .iter()
            .find(|alert| alert.kind == AlertKind::UnfilledEducation)
            .expect("education");
        let farm_rank = edu
            .mitigations
            .iter()
            .find(|mit| mit.title.to_ascii_lowercase().contains("farm"))
            .map(|mit| mit.rank)
            .expect("farm feeder");
        let mine_rank = edu
            .mitigations
            .iter()
            .find(|mit| mit.title.to_ascii_lowercase().contains("mine"))
            .map(|mit| mit.rank)
            .expect("mine feeder");
        let uni_rank = edu
            .mitigations
            .iter()
            .find(|mit| mit.title.to_ascii_lowercase().contains("university"))
            .map(|mit| mit.rank);
        assert!(farm_rank < uni_rank.unwrap_or(u32::MAX));
        assert!(mine_rank < uni_rank.unwrap_or(u32::MAX));
        assert!(
            !edu.mitigations.iter().any(is_university_build),
            "unstaffed university must not recommend another campus: {:?}",
            edu.mitigations
        );
    }

    #[test]
    fn underemployed_from_pops_does_not_lead_with_extra_levels() {
        let (world, defs, prices) = fixture();
        let result = alerts(&world, &defs, &prices);
        let farm_under = result
            .alerts
            .iter()
            .find(|alert| alert.kind == AlertKind::Underemployed && alert.building_id == Some(3))
            .expect("farm underemployed");
        let first = farm_under.mitigations.first().expect("mitigation");
        let extra = matches!(
            first.action,
            Some(MitigationAction::Build {
                extra_levels: Some(n),
                ..
            }) if n > 0
        ) && first.title.to_ascii_lowercase().contains("level");
        assert!(
            !extra,
            "pop underemployment led with extra levels: {}",
            first.title
        );
        let pops = result
            .alerts
            .iter()
            .find(|alert| alert.kind == AlertKind::UnfilledPops && alert.building_id == Some(3))
            .expect("unfilled pops on farm");
        assert_eq!(pops.mitigations[0].rank, 1);
        assert!(pops.evidence.iter().any(|row| row.label == "In-migration"));
    }

    #[test]
    fn shortage_mitigations_include_estimated_effect() {
        let (world, defs, prices) = fixture();
        let result = alerts(&world, &defs, &prices);
        assert!(result.limitations.iter().any(|line| {
            line.to_ascii_lowercase().contains("pop demand")
                && line.to_ascii_lowercase().contains("building io")
        }));
        let grain = result
            .alerts
            .iter()
            .find(|alert| {
                alert.kind == AlertKind::GoodsShortage && alert.good_id.as_deref() == Some("grain")
            })
            .expect("grain shortage");
        assert!(
            grain
                .mitigations
                .iter()
                .all(|mit| mit.effect.as_ref().is_some_and(|text| !text.is_empty())),
            "every grain mitigation needs an effect, got {:?}",
            grain
                .mitigations
                .iter()
                .map(|mit| (&mit.title, &mit.effect))
                .collect::<Vec<_>>()
        );
        let levels = grain
            .mitigations
            .iter()
            .find(|mit| mit.title.to_ascii_lowercase().contains("trade-center"))
            .expect("trade-center levels");
        let effect = levels.effect.as_deref().expect("effect");
        assert!(
            effect.contains("0 change to grain") && effect.contains("pop demand held"),
            "trade-center extra levels should not invent grain via traded_quantity, got {effect}"
        );
        let pm = grain
            .mitigations
            .iter()
            .find(|mit| matches!(mit.action, Some(MitigationAction::Pm { .. })))
            .expect("specific PM upgrade");
        assert_eq!(pm.title, "Switch to Soil Enriching Farming");
        match &pm.action {
            Some(MitigationAction::Pm {
                building_id,
                production_method,
                methods,
            }) => {
                assert_eq!(*building_id, 3);
                assert_eq!(production_method, "pm_soil_enriching_farming");
                assert_eq!(methods, &vec!["pm_soil_enriching_farming".to_string()]);
            }
            other => panic!("expected Pm action, got {other:?}"),
        }
        let pm_effect = pm.effect.as_deref().expect("pm effect");
        assert!(
            pm_effect.contains("+15 grain sell") && pm_effect.contains("pop demand held"),
            "PM local solve should add recipe grain, got {pm_effect}"
        );
        let realloc = grain
            .mitigations
            .iter()
            .find(|mit| matches!(mit.action, Some(MitigationAction::TradeAlloc { .. })))
            .expect("reallocate");
        assert!(
            realloc
                .effect
                .as_deref()
                .is_some_and(|text| text.contains("0 extra grain")),
            "reallocate should be zero in this model, got {:?}",
            realloc.effect
        );

        let power = result
            .alerts
            .iter()
            .find(|alert| alert.kind == AlertKind::ElectricityShortage)
            .expect("electricity");
        assert!(power
            .mitigations
            .iter()
            .all(|mit| mit.effect.as_ref().is_some_and(|text| !text.is_empty())));
        assert!(power.mitigations.iter().any(|mit| mit
            .effect
            .as_deref()
            .is_some_and(|text| text.contains("local-only"))));
        assert!(
            !power
                .mitigations
                .iter()
                .any(|mit| matches!(mit.action, Some(MitigationAction::Pm { .. }))),
            "electricity should not get a generic/unrelated PM"
        );
    }
}
