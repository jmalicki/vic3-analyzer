//! Shortage alerts and ranked mitigations from a solved market.
//!
//! Detection uses an existing [`PricesResult`] plus the [`World`] it came from.
//! Heuristics are documented next to each detector; they are **not** a full Vic3
//! simulation and do not re-run the NLS (mitigation `effect` strings are IO /
//! `traded_quantity` estimates).
//!
//! Qualification advice is filtered from `common/pop_types` (or a bundled
//! vanilla snapshot) plus the state's current mix. Player-facing copy lives in
//! `advice/qualifications/`.
//!
//! `vic3-api` / CLI / wasm serialize [`AlertsResult`]; SQL `alerts()` and MCP
//! hosts consume the same JSON shape.
//!
//! When mitigations are enabled, one [`MitigationIndex`] is built per
//! [`alerts_with`] / [`goods_shortage_alerts`] call so shortage lever selection
//! does not rescan every building for each alert.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vic3_defs::GameDefs;

use crate::label::{pretty_id, script_label};
use crate::qualification_advice::{
    blocking_profession, building_staffing, id_has, is_fully_staffed, is_subsistence,
    select_levers, shortage_target, state_mix, state_profession_totals, BuildingStaffing, LeverKey,
    StateMix,
};
use crate::result::{BuildingEconomics, ExtraLevelsDelta, ProductionMethodDelta, WorldDelta};
use crate::{
    apply_delta, market_access, price, GoodPrice, PricesResult, StateNeed, World, WorldBuilding,
    ORDER_EPS,
};

/// Market access at or below this ratio is a [`AlertKind::LowMarketAccess`].
const ACCESS_ALERT: f64 = 0.95;

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
    pub good_name: Option<String>,
    pub evidence: Vec<Evidence>,
    pub mitigations: Vec<Mitigation>,
    /// Buildings under a state-level employment alert. Empty elsewhere.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub staffing: Vec<BuildingStaffing>,
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
        building_type_name: String,
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
        good_name: String,
    },
    FeederJob {
        building_type_name: String,
        profession: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        state_id: Option<u32>,
    },
    SolGoods {
        good_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        state_id: Option<u32>,
    },
}

/// Options for [`alerts_with`] / SQL projection-aware scans.
///
/// When `with_mitigations` is false, detectors still emit titles/summaries/evidence
/// but skip expensive mitigation builders (world clones, lever ranking).
///
/// When `mitigation_ids` is `Some`, only those alert ids get mitigation lists
/// (detectors still run for every alert). Used by SQL filter/LIMIT pushdown so
/// agents do not pay for tens of thousands of discarded mitigations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertsOptions {
    /// Attach ranked [`Mitigation`] lists (default true for CLI/wasm parity).
    pub with_mitigations: bool,
    /// If set, only build mitigations for these alert ids. Ignored when
    /// `with_mitigations` is false. `None` means every alert (default).
    pub mitigation_ids: Option<BTreeSet<String>>,
}

impl Default for AlertsOptions {
    fn default() -> Self {
        Self {
            with_mitigations: true,
            mitigation_ids: None,
        }
    }
}

impl AlertsOptions {
    /// Whether mitigation builders should run for `alert_id`.
    pub fn wants_mitigations(&self, alert_id: &str) -> bool {
        self.with_mitigations
            && self
                .mitigation_ids
                .as_ref()
                .is_none_or(|ids| ids.contains(alert_id))
    }
}

/// Grouped arguments for alert detector collectors (`collect_*`).
struct AlertCollectArgs<'a> {
    world: &'a World,
    defs: &'a GameDefs,
    prices: &'a PricesResult,
    opts: &'a AlertsOptions,
    alerts: &'a mut Vec<Alert>,
    /// Extra caveats appended by detectors that need them.
    limitations: &'a mut BTreeSet<String>,
}

/// Grouped arguments for [`push_state_employment_alert`].
///
/// Shares the same world/defs/prices/opts/alerts fields as [`AlertCollectArgs`]
/// (flatten at the call site — do not nest `&mut AlertCollectArgs` inside this type).
struct PushEmploymentAlertArgs<'a> {
    world: &'a World,
    defs: &'a GameDefs,
    prices: &'a PricesResult,
    opts: &'a AlertsOptions,
    alerts: &'a mut Vec<Alert>,
    state_id: u32,
    mix: &'a StateMix,
    buildings: &'a [&'a BuildingEconomics],
    kind: AlertKind,
    severity: u8,
    qual_short: bool,
}

/// Diagnose shortages from a solved market. Does not mutate `world` or `prices`.
///
/// Needs-unmet uses a documented heuristic: a state need is unmet when any
/// basket good has local sell below demanded quantity, or local/market prices
/// sit at the `base * (1 + PRICE_RANGE)` ceiling.
///
/// # Arguments
///
/// * `world` — same snapshot that produced `prices` (staffing, states, trade).
/// * `defs` — goods / buildings / pop_types for detectors and lever selection.
/// * `prices` — prior [`crate::solve`] (or preview) output; local + market rows.
///
/// # Returns
///
/// [`AlertsResult`] with ranked alerts. `limitations` starts from the solve’s
/// list and may gain detector-specific caveats. No Rust `Err` — empty `alerts`
/// means nothing fired.
pub fn alerts(world: &World, defs: &GameDefs, prices: &PricesResult) -> AlertsResult {
    alerts_with(world, defs, prices, AlertsOptions::default())
}

/// Like [`alerts`], with control over mitigation expansion.
pub fn alerts_with(
    world: &World,
    defs: &GameDefs,
    prices: &PricesResult,
    opts: AlertsOptions,
) -> AlertsResult {
    let mut alerts = Vec::new();
    let mut extra_limitations = BTreeSet::new();
    let mut args = AlertCollectArgs {
        world,
        defs,
        prices,
        opts: &opts,
        alerts: &mut alerts,
        limitations: &mut extra_limitations,
    };

    collect_goods_alerts(&mut args);
    collect_needs_unmet(&mut args);
    collect_market_access(&mut args);
    collect_education(&mut args);
    collect_pop_and_underemployed(&mut args);

    finish_alerts(prices, alerts, extra_limitations)
}

/// Goods / electricity / transportation shortage alerts only (no education/pops).
///
/// Used by SQL `shortage_analysis` so agents do not pay for thousands of
/// qualification mitigations when ranking scarce goods.
pub fn goods_shortage_alerts(
    world: &World,
    defs: &GameDefs,
    prices: &PricesResult,
    opts: AlertsOptions,
) -> AlertsResult {
    let mut alerts = Vec::new();
    let mut extra_limitations = BTreeSet::new();
    collect_goods_alerts(&mut AlertCollectArgs {
        world,
        defs,
        prices,
        opts: &opts,
        alerts: &mut alerts,
        limitations: &mut extra_limitations,
    });
    finish_alerts(prices, alerts, extra_limitations)
}

fn finish_alerts(
    prices: &PricesResult,
    alerts: Vec<Alert>,
    extra_limitations: BTreeSet<String>,
) -> AlertsResult {
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

fn collect_goods_alerts(args: &mut AlertCollectArgs<'_>) {
    let AlertCollectArgs {
        world,
        defs,
        prices,
        opts,
        alerts,
        limitations,
    } = args;
    let mut seen = BTreeSet::new();
    let mut short_goods: BTreeMap<String, GoodsShortageHint> = BTreeMap::new();

    for row in &prices.goods {
        if good_is_short(row, defs) {
            short_goods.insert(
                row.name.clone(),
                GoodsShortageHint::from_row(row, None, None),
            );
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
                    let row = prices.goods.iter().find(|good| good.name == *good_id);
                    GoodsShortageHint {
                        buy: row.map(|g| g.buy).unwrap_or(0.0),
                        sell: row.map(|g| g.sell).unwrap_or(0.0),
                        price: row.map(|g| g.price).unwrap_or(0.0),
                        base: row.map(|g| g.base).unwrap_or(0.0),
                        name: row.and_then(|g| g.label.clone()),
                        building_id: Some(building.id),
                        state_id: building.state_id,
                        from_short_inputs: true,
                    }
                });
        }
    }

    // Build once per alerts pass; lean (`with_mitigations: false`) skips entirely.
    let index = opts
        .with_mitigations
        .then(|| MitigationIndex::build(world, defs, prices));

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
        let alert_id = format!("{}:{good_id}", kind_id(kind));
        let mitigations = match (opts.wants_mitigations(&alert_id), index.as_ref()) {
            (true, Some(index)) => goods_mitigations(
                &ShortageEffect {
                    world,
                    defs,
                    prices,
                    index,
                    good_name: &good_id,
                    buy: hint.buy,
                    sell: hint.sell,
                    base: hint.base,
                },
                hint.state_id,
                tradeable,
                limitations,
            ),
            _ => Vec::new(),
        };
        alerts.push(Alert {
            id: alert_id,
            kind,
            severity: 1,
            title: format!("{display} shortage"),
            summary: goods_summary(kind, tradeable, &hint),
            state_id: hint.state_id,
            building_id: hint.building_id,
            good_name: Some(good_id),
            evidence,
            mitigations,
            staffing: Vec::new(),
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
            name: row.label.clone(),
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
    let good_id = ctx.good_name;
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
                    building_type_name: "building_trade_center".into(),
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
                        building_type_name: center.building_type_name.clone(),
                        state_id: center.state_id,
                        extra_levels: Some(1),
                    },
                ),
                ctx.extra_levels(&center.building_type_name, center.state_id.or(state_id), 1),
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
                        format!(
                            "{} is not importing {good_id}. Reallocation is advice only (frozen trade).",
                            state_label(prices, world, defs, state)
                        ),
                        MitigationAction::TradeAlloc {
                            state_id: state,
                            good_name: good_id.into(),
                        },
                    ),
                    format!(
                        "0 extra {} in this model (trade volumes are frozen).",
                        ctx.good_name
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
    let building = ctx
        .index
        .local_producer_type(ctx.good_name, state_id)
        .unwrap_or_else(|| format!("building_{}_producer", ctx.good_name));
    items.push(with_effect(
        action_mit(
            format!("{alert_id}:local-producer"),
            "Expand a local producer",
            format!(
                "Raise local {} output as an alternative to trade.",
                ctx.good_name
            ),
            MitigationAction::Build {
                building_type_name: building.clone(),
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
    let Some(pick) = ctx.index.best_pm_upgrade(ctx.good_name, state_id) else {
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
                pick.building_type_name, pick.building_id
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct PmPick {
    building_id: u32,
    building_type_name: String,
    from: Vec<String>,
    to: Vec<String>,
    new_pm: String,
}

type BestPmCache = BTreeMap<(String, Option<u32>), Option<PmPick>>;

/// Structural indexes + memoization reused while expanding shortage mitigations
/// for one [`alerts_with`] / [`goods_shortage_alerts`] call.
///
/// Built only when `with_mitigations` is true so the lean projection path stays
/// cheap. Lookups preserve world / prices scan order so PM picks and local
/// producer type ids match the previous full-scan semantics.
struct MitigationIndex<'a> {
    world: &'a World,
    defs: &'a GameDefs,
    prices: &'a PricesResult,
    /// World building indices by type id (ascending = world order).
    by_type: BTreeMap<&'a str, Vec<usize>>,
    /// World building indices by `(type_id, state)`.
    #[allow(dead_code)] // available for type×state lookups; exercised in unit tests
    by_type_state: BTreeMap<(&'a str, Option<u32>), Vec<usize>>,
    /// World building indices by state (`None` key = buildings with no state).
    by_state: BTreeMap<Option<u32>, Vec<usize>>,
    /// World building indices whose current IO involves each good (by goods_order slot).
    by_good_io: Vec<Vec<usize>>,
    /// Prices-result building indices that list each good in `outputs`.
    price_output_by_good: BTreeMap<&'a str, Vec<usize>>,
    /// Cached `type_pm_candidates` results for this pass.
    pm_candidates: RefCell<BTreeMap<String, Vec<String>>>,
    /// Cached best PM upgrade per `(good_id, state_id)`.
    best_pm: RefCell<BestPmCache>,
}

impl<'a> MitigationIndex<'a> {
    fn build(world: &'a World, defs: &'a GameDefs, prices: &'a PricesResult) -> Self {
        let mut by_type: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        let mut by_type_state: BTreeMap<(&str, Option<u32>), Vec<usize>> = BTreeMap::new();
        let mut by_state: BTreeMap<Option<u32>, Vec<usize>> = BTreeMap::new();
        let mut by_good_io = vec![Vec::new(); defs.goods_order.len()];

        for (i, building) in world.buildings.iter().enumerate() {
            let type_name = building.type_script_id(defs);
            by_type.entry(type_name).or_default().push(i);
            by_type_state
                .entry((type_name, building.state))
                .or_default()
                .push(i);
            by_state.entry(building.state).or_default().push(i);

            let (inputs, outputs) = building.goods_io(defs);
            for (good, _qty) in inputs
                .iter_indexed()
                .chain(outputs.iter_indexed())
                .filter(|(_, qty)| *qty > ORDER_EPS)
            {
                let slot = good.as_usize();
                if slot < by_good_io.len() {
                    let list = &mut by_good_io[slot];
                    if list.last().copied() != Some(i) {
                        list.push(i);
                    }
                }
            }
        }

        let mut price_output_by_good: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (i, building) in prices.buildings.iter().enumerate() {
            for flow in &building.outputs {
                let list = price_output_by_good.entry(flow.name.as_str()).or_default();
                if list.last().copied() != Some(i) {
                    list.push(i);
                }
            }
        }

        Self {
            world,
            defs,
            prices,
            by_type,
            by_type_state,
            by_state,
            by_good_io,
            price_output_by_good,
            pm_candidates: RefCell::new(BTreeMap::new()),
            best_pm: RefCell::new(BTreeMap::new()),
        }
    }

    fn buildings_in_state(&self, state_id: Option<u32>) -> Vec<usize> {
        match state_id {
            None => (0..self.world.buildings.len()).collect(),
            Some(sid) => self.by_state.get(&Some(sid)).cloned().unwrap_or_default(),
        }
    }

    /// World building indices of `type_id` in `state` (empty when none). Preserves world order.
    #[cfg(test)]
    fn buildings_of_type_in_state(&self, type_id: &str, state: Option<u32>) -> Vec<usize> {
        self.by_type_state
            .get(&(type_id, state))
            .cloned()
            .unwrap_or_default()
    }

    fn type_pm_candidates(&self, type_id: &str) -> Vec<String> {
        if let Some(cached) = self.pm_candidates.borrow().get(type_id) {
            return cached.clone();
        }
        let mut ids = BTreeSet::new();
        if let Some(indices) = self.by_type.get(type_id) {
            for &i in indices {
                ids.extend(self.world.buildings[i].production_methods.iter().cloned());
            }
        }
        let out: Vec<String> = ids.into_iter().collect();
        self.pm_candidates
            .borrow_mut()
            .insert(type_id.to_string(), out.clone());
        out
    }

    fn local_producer_type(&self, good_id: &str, state_id: Option<u32>) -> Option<String> {
        if let Some(indices) = self.price_output_by_good.get(good_id) {
            for &i in indices {
                let building = &self.prices.buildings[i];
                if state_id.is_none_or(|sid| building.state_id == Some(sid)) {
                    return Some(building.building_type_name.clone());
                }
            }
        }
        let good_id = self.defs.index_of(good_id)?;
        let slot = good_id.as_usize();
        let indices = self.by_good_io.get(slot)?;
        for &i in indices {
            let row = &self.world.buildings[i];
            if state_id.is_some_and(|sid| row.state != Some(sid)) {
                continue;
            }
            return Some(row.type_script_id(self.defs).to_string());
        }
        None
    }

    fn best_pm_upgrade(&self, good_id: &str, state_id: Option<u32>) -> Option<PmPick> {
        let key = (good_id.to_string(), state_id);
        if let Some(cached) = self.best_pm.borrow().get(&key) {
            return cached.clone();
        }
        let pick = self.best_pm_upgrade_uncached(good_id, state_id);
        self.best_pm.borrow_mut().insert(key, pick.clone());
        pick
    }

    fn best_pm_upgrade_uncached(&self, good_id: &str, state_id: Option<u32>) -> Option<PmPick> {
        let idx = self.defs.index_of(good_id)?;
        let mut best: Option<PmPick> = None;
        let mut best_score = ORDER_EPS;
        for i in self.buildings_in_state(state_id) {
            let building = &self.world.buildings[i];
            let current = &building.production_methods;
            if current.is_empty() {
                continue;
            }
            let candidates = self.type_pm_candidates(building.type_script_id(self.defs));
            if candidates.len() < 2 {
                continue;
            }
            let (in0, out0) = building.goods_io(self.defs);
            for slot in 0..current.len() {
                for candidate in &candidates {
                    if current[slot] == *candidate {
                        continue;
                    }
                    let mut methods = current.clone();
                    methods[slot] = candidate.clone();
                    let trial = building.with_methods(methods.clone());
                    let (in1, out1) = trial.goods_io(self.defs);
                    let score = (out1[idx] - in1[idx]) - (out0[idx] - in0[idx]);
                    if score > best_score {
                        best_score = score;
                        best = Some(PmPick {
                            building_id: building.id,
                            building_type_name: building.type_script_id(self.defs).to_string(),
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
    index: &'a MitigationIndex<'a>,
    good_name: &'a str,
    buy: f64,
    sell: f64,
    base: f64,
}

impl ShortageEffect<'_> {
    fn gap(&self) -> f64 {
        (self.buy - self.sell).max(0.0)
    }

    fn extra_levels(&self, type_id: &str, state_id: Option<u32>, extra: u32) -> String {
        self.effect_from_delta(&extra_levels_on_type(
            self.world, self.defs, type_id, state_id, extra,
        ))
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
        let (buy0, sell0) = good_io_total(self.world, self.defs, self.good_name);
        let (buy1, sell1) = good_io_total(next, self.defs, self.good_name);
        let dbuy = buy1 - buy0;
        let dsell = sell1 - sell0;
        let new_buy = (self.buy + dbuy).max(0.0);
        let new_sell = (self.sell + dsell).max(0.0);
        let old_price = price(self.base, self.buy, self.sell, self.defs.price_range);
        let new_price = price(self.base, new_buy, new_sell, self.defs.price_range);
        format_local_effect(
            self.good_name,
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
    defs: &GameDefs,
    type_id: &str,
    state_id: Option<u32>,
    extra: u32,
) -> WorldDelta {
    let want = defs.building_index_of(type_id);
    let extra_levels = world
        .buildings
        .iter()
        .filter(|building| {
            want == Some(building.building_type_id)
                && state_id.is_none_or(|sid| building.state == Some(sid))
        })
        .map(|building| ExtraLevelsDelta {
            building_type_id: None,
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
    good_name: &str,
    dsell: f64,
    dbuy: f64,
    old_price: f64,
    new_price: f64,
    old_gap: f64,
    new_gap: f64,
) -> String {
    let orders = if dsell.abs() <= ORDER_EPS && dbuy.abs() <= ORDER_EPS {
        format!("0 change to {good_name} building orders")
    } else {
        format!(
            "{} {good_name} sell, {} {good_name} buy",
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

fn collect_needs_unmet(args: &mut AlertCollectArgs<'_>) {
    let AlertCollectArgs {
        world,
        defs,
        prices,
        opts,
        alerts,
        limitations,
    } = args;
    // Heuristic: a state need is unmet when any basket good has local sell
    // below the demanded quantity, or local/market prices sit at the
    // `base * (1 + PRICE_RANGE)` ceiling.
    let ceiling_factor = 1.0 + defs.price_range.max(0.0);
    let mut by_state: BTreeMap<u32, Vec<&StateNeed>> = BTreeMap::new();
    for need in &prices.state_needs {
        if !need_trips(need, prices, defs, ceiling_factor).is_empty() {
            by_state.entry(need.state_id).or_default().push(need);
        }
    }
    if by_state.is_empty() {
        return;
    }
    limitations.insert(
        "Needs-unmet detection compares need Amounts to local Sell, and flags goods at the max price."
            .into(),
    );
    for (state_id, needs) in by_state {
        let mut evidence = Vec::new();
        // `(good_name, good_label)` — script key + localized display for mitigations.
        let mut goods: Vec<(String, String)> = Vec::new();
        for need in &needs {
            for trip in need_trips(need, prices, defs, ceiling_factor) {
                evidence.push(Evidence {
                    label: format!("{}: {}", trip.need_label, trip.good_label),
                    value: trip.value,
                });
                if !goods.iter().any(|(name, _)| name == &trip.good_name) {
                    goods.push((trip.good_name, trip.good_label));
                }
            }
        }
        let good_name = goods.first().map(|(name, _)| name.clone());
        let alert_id = format!("needs_unmet:{state_id}");
        let mitigations = if opts.wants_mitigations(&alert_id) {
            need_mitigations(state_id, &goods)
        } else {
            Vec::new()
        };
        alerts.push(Alert {
            id: alert_id,
            kind: AlertKind::NeedsUnmet,
            severity: 1,
            title: format!(
                "Unmet pop needs in {}",
                state_label(prices, world, defs, state_id)
            ),
            summary: "Need goods exceed local sell, or sit at the max price.".into(),
            state_id: Some(state_id),
            building_id: None,
            good_name,
            evidence,
            mitigations,
            staffing: Vec::new(),
        });
    }
}

/// Private UI-facing row for needs-unmet evidence. Labels are display strings;
/// `good_name` is the script key used for actions and dedup.
struct NeedTrip {
    need_label: String,
    good_name: String,
    good_label: String,
    value: String,
}

fn need_trips(
    need: &StateNeed,
    prices: &PricesResult,
    defs: &GameDefs,
    ceiling_factor: f64,
) -> Vec<NeedTrip> {
    if need.goods.is_empty() && need.package_value > 0.0 {
        return Vec::new();
    }
    let need_label = need
        .label
        .as_deref()
        .filter(|label| !label.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| script_label(defs, &need.name));
    let mut trips = Vec::new();
    for flow in &need.goods {
        if flow.quantity <= ORDER_EPS {
            continue;
        }
        let local = prices
            .state_goods
            .iter()
            .find(|row| row.state_id == need.state_id && row.name == flow.name);
        let market = prices.goods.iter().find(|good| good.name == flow.name);
        let amount_short = match local {
            Some(local) => local.sell + ORDER_EPS < flow.quantity,
            None => true,
        };
        let at_max = local
            .map(|row| row.price + ORDER_EPS >= row.base * ceiling_factor)
            .unwrap_or(false)
            || market
                .map(|row| row.price + ORDER_EPS >= row.base * ceiling_factor)
                .unwrap_or(false);
        if !amount_short && !at_max {
            continue;
        }
        let good_label = resolve_good_label(prices, defs, &flow.name);
        let value = if amount_short {
            let sell = local.map(|row| row.sell).unwrap_or(0.0);
            format!("{} vs sell {}", format_num(flow.quantity), format_num(sell))
        } else {
            let (price, base) = if let Some(local) = local {
                if local.price + ORDER_EPS >= local.base * ceiling_factor {
                    (local.price, local.base)
                } else if let Some(market) = market {
                    (market.price, market.base)
                } else {
                    (local.price, local.base)
                }
            } else if let Some(market) = market {
                (market.price, market.base)
            } else {
                continue;
            };
            format!("{} / {} (max)", format_num(price), format_num(base))
        };
        trips.push(NeedTrip {
            need_label: need_label.clone(),
            good_name: flow.name.clone(),
            good_label,
            value,
        });
    }
    trips
}

fn resolve_good_label(prices: &PricesResult, defs: &GameDefs, name: &str) -> String {
    prices
        .goods
        .iter()
        .find(|good| good.name == name)
        .and_then(|good| good.label.clone())
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| script_label(defs, name))
}

fn need_mitigations(state_id: u32, goods: &[(String, String)]) -> Vec<Mitigation> {
    let mut items = Vec::new();
    for (good_name, good_label) in goods.iter().take(3) {
        items.push(action_mit(
            format!("needs:{state_id}:sol:{good_name}"),
            format!("Cheapen {good_label}"),
            format!(
                "Lower the local price of {good_label} (produce more or import) to cover the need basket."
            ),
            MitigationAction::SolGoods {
                good_name: good_name.clone(),
                state_id: Some(state_id),
            },
        ));
        items.push(action_mit(
            format!("needs:{state_id}:import:{good_name}"),
            format!("Import {good_label} through a trade center"),
            format!("Trade-center imports of pop goods can fill the {good_label} basket."),
            MitigationAction::TradeAlloc {
                state_id,
                good_name: good_name.clone(),
            },
        ));
    }
    rank(items)
}

fn collect_market_access(args: &mut AlertCollectArgs<'_>) {
    let AlertCollectArgs {
        world,
        defs,
        prices,
        opts,
        alerts,
        limitations: _,
    } = args;
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
        let alert_id = format!("low_market_access:{}", state.id);
        let mitigations = if opts.wants_mitigations(&alert_id) {
            rank(vec![action_mit(
                format!("access:{}:rail", state.id),
                "Add infrastructure",
                "Build railways or urban infrastructure so usage no longer exceeds capacity.",
                MitigationAction::Build {
                    building_type_name: "building_railway".into(),
                    state_id: Some(state.id),
                    extra_levels: Some(1),
                },
            )])
        } else {
            Vec::new()
        };
        alerts.push(Alert {
            id: alert_id,
            kind: AlertKind::LowMarketAccess,
            severity: 1,
            title: format!(
                "Low market access in {}",
                state_label(prices, world, defs, state.id)
            ),
            summary: format!(
                "Infrastructure {} / usage {} is {:.0}% (threshold {:.0}%).",
                format_num(infra),
                format_num(usage),
                access * 100.0,
                ACCESS_ALERT * 100.0
            ),
            state_id: Some(state.id),
            building_id: None,
            good_name: None,
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
            mitigations,
            staffing: Vec::new(),
        });
    }
}

struct AccessState {
    id: u32,
    infrastructure: Option<f64>,
    infrastructure_usage: Option<f64>,
}

fn collect_education(args: &mut AlertCollectArgs<'_>) {
    let AlertCollectArgs {
        world,
        defs,
        prices,
        opts,
        alerts,
        limitations: _,
    } = args;
    for row in &prices.state_qualifications {
        if row.shortage <= ORDER_EPS {
            continue;
        }
        let target = row.name.as_str();
        let mix = state_mix(prices, row.state_id);
        let alert_id = format!("unfilled_education:{}:{target}", row.state_id);
        let mitigations = if opts.wants_mitigations(&alert_id) {
            let mut items = qualification_levers(prices, defs, row.state_id, target, &mix);
            let wants_university = items.iter().any(is_university_build);
            if wants_university && has_unstaffed_university(prices, world, defs, row.state_id) {
                items.retain(|item| !is_university_build(item));
                items.push(plain(
                    format!("edu:{}:staff-uni", row.state_id),
                    "Staff the university already in this state",
                    "A campus here is not fully employed. Filling those academic jobs raises literacy and qualifications; another empty campus will not.",
                ));
            }
            rank(items)
        } else {
            Vec::new()
        };
        let profession = display_prof(defs, row);
        let place = state_label(prices, world, defs, row.state_id);
        let stock = row.employable.unwrap_or(row.qualified);
        alerts.push(Alert {
            id: alert_id,
            kind: AlertKind::UnfilledEducation,
            severity: 1,
            title: format!("{place} needs {} more {profession}", format_num(row.shortage)),
            summary: format!(
                "{place} has {} {profession} jobs and {} people who can take them. Need {} more qualified {profession}.",
                format_num(row.jobs),
                format_num(stock),
                format_num(row.shortage)
            ),
            state_id: Some(row.state_id),
            building_id: None,
            good_name: None,
            evidence: vec![
                Evidence {
                    label: "State".into(),
                    value: place,
                },
                Evidence {
                    label: "Profession".into(),
                    value: profession,
                },
                Evidence {
                    label: "Jobs in this state".into(),
                    value: format_num(row.jobs),
                },
                Evidence {
                    label: "People who can take those jobs".into(),
                    value: format_num(stock),
                },
                Evidence {
                    label: "Still needed".into(),
                    value: format_num(row.shortage),
                },
                Evidence {
                    label: "Literacy".into(),
                    value: format!("{:.0}%", mix.literacy * 100.0),
                },
                Evidence {
                    label: "Who lives here".into(),
                    value: mix.summary(),
                },
            ],
            mitigations,
            staffing: Vec::new(),
        });
    }
}

fn collect_pop_and_underemployed(args: &mut AlertCollectArgs<'_>) {
    let AlertCollectArgs {
        world,
        defs,
        prices,
        opts,
        alerts,
        limitations,
    } = args;
    let buildings = if prices.buildings.is_empty() {
        world
            .buildings
            .iter()
            .map(|b| economics_from_world(b, defs))
            .collect::<Vec<_>>()
    } else {
        prices.buildings.clone()
    };
    let mut by_state: BTreeMap<u32, Vec<&BuildingEconomics>> = BTreeMap::new();
    for building in &buildings {
        if building.level <= ORDER_EPS || is_fully_staffed(building.staffing, building.level) {
            continue;
        }
        let Some(state_id) = building.state_id else {
            continue;
        };
        by_state.entry(state_id).or_default().push(building);
    }
    for (state_id, group) in by_state {
        let mix = state_mix(prices, state_id);
        let mut qual_buildings = Vec::new();
        let mut pop_buildings = Vec::new();
        for building in group {
            if has_employee_qual_shortage(prices, building) {
                qual_buildings.push(building);
            } else {
                pop_buildings.push(building);
            }
        }
        if !pop_buildings.is_empty() {
            limitations.insert(
                "This tool does not simulate pops moving between states. Empty building levels will not attract migrants here."
                    .into(),
            );
            push_state_employment_alert(&mut PushEmploymentAlertArgs {
                world,
                defs,
                prices,
                opts,
                alerts,
                state_id,
                mix: &mix,
                buildings: &pop_buildings,
                kind: AlertKind::UnfilledPops,
                severity: 1,
                qual_short: false,
            });
        }
        if !qual_buildings.is_empty() {
            push_state_employment_alert(&mut PushEmploymentAlertArgs {
                world,
                defs,
                prices,
                opts,
                alerts,
                state_id,
                mix: &mix,
                buildings: &qual_buildings,
                kind: AlertKind::Underemployed,
                severity: 2,
                qual_short: true,
            });
        }
    }
}

fn push_state_employment_alert(args: &mut PushEmploymentAlertArgs<'_>) {
    let PushEmploymentAlertArgs {
        world,
        defs,
        prices,
        opts,
        alerts,
        state_id,
        mix,
        buildings,
        kind,
        severity,
        qual_short,
    } = args;
    let state_id = *state_id;
    let kind = *kind;
    let severity = *severity;
    let qual_short = *qual_short;
    let place = state_label(prices, world, defs, state_id);
    let staffing: Vec<BuildingStaffing> = buildings
        .iter()
        .map(|building| {
            building_staffing(
                defs,
                prices,
                building,
                building_label(prices, defs, &building.building_type_name),
            )
        })
        .collect();
    let totals = state_profession_totals(&staffing);
    let blocker = blocking_profession(&staffing);
    let blocker_name = blocker.map(|row| row.label.clone().unwrap_or_else(|| pretty_id(&row.name)));
    let building_count = staffing.len();
    let title = if qual_short {
        match blocker_name.as_deref() {
            Some(name) => format!("{place} buildings cannot fill {name} jobs"),
            None => format!("Understaffed buildings in {place}"),
        }
    } else {
        format!("Not enough people for jobs in {place}")
    };
    let summary = if qual_short {
        match blocker {
            Some(row) => format!(
                "{place} is short {} {}. {} building{} below full staffing. Extra levels add more empty jobs; the qualification shortage for this state has the next steps.",
                format_num(row.state_shortage.max(row.missing_here)),
                row.label
                    .as_deref()
                    .unwrap_or(&row.name),
                building_count,
                if building_count == 1 { "" } else { "s" }
            ),
            None => format!(
                "{building_count} building{} in {place} are below full staffing because of missing qualifications.",
                if building_count == 1 { "" } else { "s" }
            ),
        }
    } else {
        format!(
            "{building_count} building{} in {place} have open jobs and enough qualified people. The state is short of workers, not diplomas.",
            if building_count == 1 { "" } else { "s" }
        )
    };
    let mut evidence = vec![
        Evidence {
            label: "State".into(),
            value: place.clone(),
        },
        Evidence {
            label: "Buildings below full staffing".into(),
            value: building_count.to_string(),
        },
        Evidence {
            label: "Who lives here".into(),
            value: mix.summary(),
        },
    ];
    for row in totals
        .iter()
        .filter(|row| row.missing_here > ORDER_EPS || row.state_shortage > ORDER_EPS)
    {
        let name = row.label.as_deref().unwrap_or(&row.name);
        evidence.push(Evidence {
            label: format!("{name} to finish these buildings"),
            value: format_num(row.missing_here),
        });
        evidence.push(Evidence {
            label: format!("{name} still needed in the whole state"),
            value: format!(
                "{} ({} jobs, {} people who can take them)",
                format_num(row.state_shortage),
                format_num(row.state_jobs),
                format_num(row.state_stock)
            ),
        });
    }
    let alert_id = format!("{}:{state_id}", kind_id(kind));
    let mitigations = if !opts.wants_mitigations(&alert_id) {
        Vec::new()
    } else if qual_short {
        let target = buildings
            .iter()
            .find_map(|building| shortage_target(prices, building))
            .map(|id| profession_label(defs, id, None))
            .unwrap_or_else(|| "workers".into());
        vec![plain(
            format!("under:{state_id}:qual"),
            format!("See the {target} qualification shortage for {place}"),
            "These buildings are waiting on qualified workers who do not exist in this state. The steps that create those qualifications are listed on that shortage, not on each mill.",
        )]
    } else {
        pop_shortage_mitigations(prices, defs, state_id, buildings.first().copied(), mix)
    };
    alerts.push(Alert {
        id: alert_id,
        kind,
        severity,
        title,
        summary,
        state_id: Some(state_id),
        building_id: None,
        good_name: None,
        evidence,
        mitigations: rank(mitigations),
        staffing,
    });
}

fn pop_shortage_mitigations(
    prices: &PricesResult,
    defs: &GameDefs,
    state_id: u32,
    sample: Option<&BuildingEconomics>,
    mix: &StateMix,
) -> Vec<Mitigation> {
    let mut items = Vec::new();
    let state = Some(state_id);
    let pop_goods = expensive_pop_goods(prices, defs, state);
    let sample_id = sample.map(|building| building.id).unwrap_or(state_id);
    for good_id in pop_goods.iter().take(2) {
        let good_name = script_label(defs, good_id);
        items.push(action_mit(
            format!("pops:{sample_id}:sol:{good_id}"),
            format!("Lower the price of {good_name}"),
            format!(
                "Pops buy {good_name} as a need. A lower price raises their standard of living, which can keep workers in this state. Adding empty building levels does not hire anyone."
            ),
            MitigationAction::SolGoods {
                good_name: good_id.clone(),
                state_id: state,
            },
        ));
        if is_tradeable(good_id, defs) {
            items.push(action_mit(
                format!("pops:{sample_id}:import:{good_id}"),
                format!("Import {good_name} through a trade center"),
                format!(
                    "Importing {good_name} can make that need cheaper. This tool cannot actually move trade yet."
                ),
                MitigationAction::TradeAlloc {
                    state_id,
                    good_name: good_id.clone(),
                },
            ));
        }
    }
    if mix.peasants() > 0.0 {
        let farm = commercial_farm(prices, defs, state).unwrap_or("building_rye_farm");
        items.push(action_mit(
            format!("pops:{sample_id}:peasants"),
            "Hire peasants into commercial farms",
            "This state still has peasants. Put them in rye or wheat farms so they become farmers and laborers. Subsistence farms already hold leftover peasants and will not fill factory jobs.",
            MitigationAction::FeederJob {
                building_type_name: farm.into(),
                profession: "farmers".into(),
                state_id: state,
            },
        ));
    }
    items.push(plain(
        format!("pops:{sample_id}:migration"),
        "This tool cannot move pops between states",
        "In the game, empty buildings can attract migrants. This analyzer does not simulate that, so extra levels will not fill jobs here.",
    ));
    items.push(plain(
        format!("pops:{sample_id}:levels"),
        "Do not add empty levels",
        "More unstaffed levels add jobs this state cannot fill.",
    ));
    items
}

fn commercial_farm<'a>(
    prices: &'a PricesResult,
    defs: &GameDefs,
    state_id: Option<u32>,
) -> Option<&'a str> {
    prices.buildings.iter().find_map(|building| {
        if state_id.is_some_and(|id| building.state_id != Some(id)) {
            return None;
        }
        if is_subsistence(&building.building_type_name, defs) {
            return None;
        }
        id_has(&building.building_type_name, "farm").then_some(building.building_type_name.as_str())
    })
}

fn qualification_levers(
    prices: &PricesResult,
    defs: &GameDefs,
    state_id: u32,
    target: &str,
    mix: &StateMix,
) -> Vec<Mitigation> {
    select_levers(prices, defs, state_id, target, mix)
        .into_iter()
        .enumerate()
        .map(|(index, lever)| {
            let id = format!("ladder:{state_id}:{}:{index}", lever.key.as_str());
            match lever.key {
                LeverKey::University => action_mit(
                    id,
                    lever.title,
                    lever.detail,
                    MitigationAction::Build {
                        building_type_name: lever
                            .building
                            .unwrap_or_else(|| "building_university".into()),
                        state_id: Some(state_id),
                        extra_levels: Some(1),
                    },
                ),
                LeverKey::Wealth => {
                    let good = expensive_pop_goods(prices, defs, Some(state_id))
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| "grain".into());
                    action_mit(
                        id,
                        lever.title,
                        lever.detail,
                        MitigationAction::SolGoods {
                            good_name: good,
                            state_id: Some(state_id),
                        },
                    )
                }
                LeverKey::AlwaysHire => plain(id, lever.title, lever.detail),
                _ => {
                    let building = lever
                        .building
                        .clone()
                        .unwrap_or_else(|| lever.key.fallback_building_owned());
                    action_mit(
                        id,
                        lever.title,
                        lever.detail,
                        MitigationAction::FeederJob {
                            building_type_name: building,
                            profession: lever.source,
                            state_id: Some(state_id),
                        },
                    )
                }
            }
        })
        .collect()
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
                .find(|good| good.name == flow.name)
                .map(|good| {
                    if good.base > 0.0 {
                        good.price / good.base
                    } else {
                        0.0
                    }
                })
                .unwrap_or(0.0);
            scored.push((flow.name.clone(), price));
        }
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.dedup_by(|a, b| a.0 == b.0);
    if scored.is_empty() {
        prices
            .goods
            .iter()
            .filter(|good| good.price + ORDER_EPS >= good.base * ceiling)
            .map(|good| good.name.clone())
            .collect()
    } else {
        scored.into_iter().map(|(id, _)| id).collect()
    }
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
            row.state_id == state_id && row.name == employee.name && row.shortage > ORDER_EPS
        })
    })
}

fn has_unstaffed_university(
    prices: &PricesResult,
    world: &World,
    defs: &GameDefs,
    state_id: u32,
) -> bool {
    prices.buildings.iter().any(|building| {
        building.state_id == Some(state_id)
            && id_has(&building.building_type_name, "university")
            && building.staffing + ORDER_EPS < building.level
    }) || world.buildings.iter().any(|building| {
        building.state == Some(state_id)
            && id_has(building.type_script_id(defs), "university")
            && building.staffing + ORDER_EPS < building.level
    })
}

fn is_university_build(item: &Mitigation) -> bool {
    match &item.action {
        Some(MitigationAction::Build {
            building_type_name, ..
        }) => id_has(building_type_name, "university"),
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
            id_has(&building.building_type_name, "trade_center")
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

fn economics_from_world(building: &WorldBuilding, defs: &GameDefs) -> BuildingEconomics {
    BuildingEconomics {
        id: building.id,
        state_id: building.state,
        building_type_id: Some(building.building_type_id),
        building_type_name: building.type_script_id(defs).to_string(),
        building_type_label: defs.labels.get(building.type_script_id(defs)).cloned(),
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

fn display_prof(defs: &GameDefs, row: &crate::StateQualification) -> String {
    profession_label(defs, &row.name, row.label.as_deref())
}

fn state_label(prices: &PricesResult, world: &World, defs: &GameDefs, state_id: u32) -> String {
    if let Some(state) = prices.states.iter().find(|state| state.id == state_id) {
        if let Some(name) = state.label.as_deref().filter(|name| !name.is_empty()) {
            return name.to_string();
        }
        if let Some(region) = state.region_name.as_deref() {
            return script_label(defs, region);
        }
    }
    if let Some(state) = world.states.iter().find(|state| state.id == state_id) {
        if let Some(region) = state.region.as_deref() {
            return script_label(defs, region);
        }
    }
    format!("state {state_id}")
}

fn building_label(prices: &PricesResult, defs: &GameDefs, type_id: &str) -> String {
    prices
        .building_types
        .iter()
        .find(|row| row.name == type_id)
        .and_then(|row| row.label.clone())
        .or_else(|| defs.labels.get(type_id).cloned())
        .unwrap_or_else(|| pretty_id(type_id))
}

fn profession_label(
    defs: &GameDefs,
    profession_name: &str,
    profession_label: Option<&str>,
) -> String {
    profession_label
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| defs.labels.get(profession_name).cloned())
        .unwrap_or_else(|| pretty_id(profession_name))
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
    use vic3_defs::{GameDefs, Good, GoodId, ProductionMethod};

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
        let grain = GoodId::from_usize(0);
        defs.production_methods.insert(
            "pm_simple_farming".into(),
            ProductionMethod {
                id: "pm_simple_farming".into(),
                inputs: Vec::new(),
                outputs: vec![(grain, 10.0)],
                ..Default::default()
            },
        );
        defs.production_methods.insert(
            "pm_soil_enriching_farming".into(),
            ProductionMethod {
                id: "pm_soil_enriching_farming".into(),
                inputs: Vec::new(),
                outputs: vec![(grain, 25.0)],
                ..Default::default()
            },
        );
        defs.labels.insert(
            "pm_soil_enriching_farming".into(),
            "Soil Enriching Farming".into(),
        );
        for kind in [
            "building_trade_center",
            "building_university",
            "building_rye_farm",
            "building_tooling_workshop",
        ] {
            defs.ensure_building_type(kind);
        }
        defs
    }

    fn good(id: &str, base: f64, price: f64, buy: f64, sell: f64) -> GoodPrice {
        GoodPrice {
            name: id.into(),
            label: Some(id.into()),
            base,
            price,
            buy,
            sell,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn econ(
        defs: &GameDefs,
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
            building_type_id: defs.building_index_of(type_id),
            building_type_name: type_id.into(),
            building_type_label: defs.labels.get(type_id).cloned(),
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
                    name: (*prof).into(),
                    label: None,
                    count: *count,
                })
                .collect(),
        }
    }

    fn world_building(
        defs: &GameDefs,
        id: u32,
        state: u32,
        kind: &str,
        level: f64,
        staffing: f64,
    ) -> WorldBuilding {
        world_building_pm(defs, id, state, kind, level, staffing, &["pm_default"])
    }

    fn world_building_pm(
        defs: &GameDefs,
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
            building_type_id: defs.building_index_of(kind).expect(kind),
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
                world_building(&defs, 1, 1, "building_trade_center", 2.0, 2.0),
                world_building(&defs, 2, 1, "building_university", 1.0, 0.0),
                world_building_pm(
                    &defs,
                    3,
                    1,
                    "building_rye_farm",
                    3.0,
                    1.0,
                    &["pm_simple_farming"],
                ),
                world_building(&defs, 4, 1, "building_tooling_workshop", 2.0, 0.5),
                world_building_pm(
                    &defs,
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
                region_name: Some("STATE_TEST".into()),
                region_label: Some("Test".into()),
                label: Some("Test".into()),
                country_id: Some(1),
                market_id: Some(1),
                arable_land: None,
                infrastructure: Some(40.0),
                infrastructure_usage: Some(100.0),
            }],
            state_goods: vec![StateGood {
                state_id: 1,
                name: "clothes".into(),
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
                econ(
                    &defs,
                    1,
                    1,
                    "building_trade_center",
                    2.0,
                    2.0,
                    &[],
                    &[],
                    15.0,
                ),
                econ(
                    &defs,
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
                        &defs,
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
                        name: "grain".into(),
                        quantity: 30.0,
                        value: 600.0,
                    });
                    farm
                },
                econ(
                    &defs,
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
                    profession_name: Some("peasants".into()),
                    profession_label: Some("Peasants".into()),
                    demand_size: Some(20_000.0),
                    workforce: Some(12_000.0),
                    dependents: Some(8_000.0),
                    wealth: Some(8),
                    culture_name: None,
                    culture_label: None,
                    literate: Some(1_000.0),
                    workplace_id: None,
                    qualifications: Vec::new(),
                    needs: Vec::new(),
                },
                StatePop {
                    state_id: 1,
                    id: Some(2),
                    profession_name: Some("farmers".into()),
                    profession_label: Some("Farmers".into()),
                    demand_size: Some(4_000.0),
                    workforce: Some(4_000.0),
                    dependents: Some(0.0),
                    wealth: Some(10),
                    culture_name: None,
                    culture_label: None,
                    literate: Some(400.0),
                    workplace_id: Some(3),
                    qualifications: Vec::new(),
                    needs: Vec::new(),
                },
            ]
            .into(),
            state_qualifications: vec![StateQualification {
                state_id: 1,
                name: "machinists".into(),
                label: Some("Machinists".into()),
                qualified: 800.0,
                employable: Some(800.0),
                employed: 800.0,
                jobs: 5_000.0,
                shortage: 4_200.0,
                monthly_change: None,
            }],
            state_needs: vec![StateNeed {
                state_id: 1,
                name: "popneed_clothing".into(),
                label: Some("Clothing".into()),
                package_value: 80.0,
                goods: vec![GoodFlow {
                    name: "clothes".into(),
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
                alert.kind == AlertKind::GoodsShortage
                    && alert.good_name.as_deref() == Some("grain")
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
        let titles: Vec<_> = edu
            .mitigations
            .iter()
            .map(|mit| mit.title.to_ascii_lowercase())
            .collect();
        assert!(
            titles.iter().any(|title| title.contains("farm")),
            "machinist shortage in a peasant state should recommend commercial farms, got {titles:?}"
        );
        assert!(
            titles.iter().any(|title| title.contains("mine")),
            "machinist shortage in a peasant state should recommend mines, got {titles:?}"
        );
        assert!(
            !titles.iter().any(|title| title.contains("university")),
            "peasants are not ready for a university to produce machinists: {titles:?}"
        );
        assert!(
            !titles.iter().any(|title| title.contains("workshop")),
            "workshops hire machinists and would add empty jobs: {titles:?}"
        );
        assert!(
            !edu.mitigations.iter().any(is_university_build),
            "must not queue another campus: {:?}",
            edu.mitigations
        );
    }

    #[test]
    fn aristocrat_shortage_skips_mines_and_university() {
        let (world, defs, mut prices) = fixture();
        prices.state_pops = vec![crate::StatePop {
            state_id: 1,
            id: Some(1),
            profession_name: Some("peasants".into()),
            profession_label: Some("Peasants".into()),
            demand_size: Some(20_000.0),
            workforce: Some(12_000.0),
            dependents: Some(8_000.0),
            wealth: Some(8),
            culture_name: None,
            culture_label: None,
            literate: Some(1_000.0),
            workplace_id: None,
            qualifications: Vec::new(),
            needs: Vec::new(),
        }]
        .into();
        prices.state_qualifications[0].name = "aristocrats".into();
        prices.state_qualifications[0].label = Some("Aristocrats".into());
        let result = alerts(&world, &defs, &prices);
        let edu = result
            .alerts
            .iter()
            .find(|alert| alert.kind == AlertKind::UnfilledEducation)
            .expect("education");
        let titles: Vec<_> = edu
            .mitigations
            .iter()
            .map(|mit| mit.title.to_ascii_lowercase())
            .collect();
        assert!(
            titles.iter().any(|title| title.contains("farm")),
            "aristocrats come from farmers, got {titles:?}"
        );
        assert!(
            !titles
                .iter()
                .any(|title| title.contains("mine") || title.contains("workshop")),
            "mines and workshops do not produce aristocrats: {titles:?}"
        );
        assert!(
            !titles.iter().any(|title| title.contains("university")),
            "university does not turn peasants into aristocrats: {titles:?}"
        );
        assert!(
            edu.summary.to_ascii_lowercase().contains("aristocrat"),
            "summary should name aristocrats, got {}",
            edu.summary
        );
    }

    #[test]
    fn engineer_shortage_with_machinists_recommends_university_not_farms() {
        let (world, defs, mut prices) = fixture();
        prices.state_pops = vec![StatePop {
            state_id: 1,
            id: Some(1),
            profession_name: Some("machinists".into()),
            profession_label: Some("Machinists".into()),
            demand_size: Some(20_000.0),
            workforce: Some(12_000.0),
            dependents: Some(8_000.0),
            wealth: Some(12),
            culture_name: None,
            culture_label: None,
            literate: Some(16_000.0),
            workplace_id: Some(4),
            qualifications: Vec::new(),
            needs: Vec::new(),
        }]
        .into();
        prices.state_qualifications[0].name = "engineers".into();
        prices.state_qualifications[0].label = Some("Engineers".into());
        let result = alerts(&world, &defs, &prices);
        let edu = result
            .alerts
            .iter()
            .find(|alert| alert.kind == AlertKind::UnfilledEducation)
            .expect("education");
        let titles: Vec<_> = edu
            .mitigations
            .iter()
            .map(|mit| mit.title.to_ascii_lowercase())
            .collect();
        assert!(
            titles.iter().any(|title| title.contains("university")),
            "engineers with machinists and literacy should get a university, got {titles:?}"
        );
        assert!(
            !titles.iter().any(|title| title.contains("farm")),
            "do not send machinists back to farms: {titles:?}"
        );
    }

    #[test]
    fn farmer_heavy_aristocrat_shortage_does_not_recommend_more_farms() {
        let (world, defs, mut prices) = fixture();
        prices.state_pops = vec![crate::StatePop {
            state_id: 1,
            id: Some(1),
            profession_name: Some("farmers".into()),
            profession_label: Some("Farmers".into()),
            demand_size: Some(20_000.0),
            workforce: Some(12_000.0),
            dependents: Some(8_000.0),
            wealth: Some(12),
            culture_name: None,
            culture_label: None,
            literate: Some(12_000.0),
            workplace_id: Some(3),
            qualifications: Vec::new(),
            needs: Vec::new(),
        }]
        .into();
        prices.state_qualifications[0].name = "aristocrats".into();
        prices.state_qualifications[0].label = Some("Aristocrats".into());
        let result = alerts(&world, &defs, &prices);
        let edu = result
            .alerts
            .iter()
            .find(|alert| alert.kind == AlertKind::UnfilledEducation)
            .expect("education");
        let titles: Vec<_> = edu
            .mitigations
            .iter()
            .map(|mit| mit.title.to_ascii_lowercase())
            .collect();
        assert!(
            !titles
                .iter()
                .any(|title| title.contains("farm") && !title.contains("farmer")),
            "farmers already exist; more farms are noise: {titles:?}"
        );
        assert!(
            titles.iter().any(|title| title.contains("wealth")
                || title.contains("university")
                || title.contains("government")),
            "expected wealth, university, or government, got {titles:?}"
        );
    }

    #[test]
    fn underemployed_from_pops_does_not_lead_with_extra_levels() {
        let (world, defs, prices) = fixture();
        let result = alerts(&world, &defs, &prices);
        let farm_under = result
            .alerts
            .iter()
            .find(|alert| {
                alert.kind == AlertKind::Underemployed
                    && alert.state_id == Some(1)
                    && alert.staffing.iter().any(|row| row.building_id == 4)
            })
            .expect("state underemployed for the workshop");
        assert!(farm_under.building_id.is_none());
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
        assert!(
            first.title.to_ascii_lowercase().contains("qualification"),
            "qualification-blocked buildings should point at the state shortage, got {}",
            first.title
        );
        let pops = result
            .alerts
            .iter()
            .find(|alert| alert.kind == AlertKind::UnfilledPops && alert.state_id == Some(1))
            .expect("unfilled pops in state");
        assert_eq!(pops.mitigations[0].rank, 1);
        assert!(pops.staffing.iter().any(|row| row.building_id == 3));
        assert!(
            pops.evidence
                .iter()
                .any(|row| row.label == "State" && row.value == "Test"),
            "unfilled pops should name the state"
        );
        assert!(
            farm_under.title.contains("Test") && !farm_under.title.contains("building_"),
            "underemployed title should name the state, got {}",
            farm_under.title
        );
        assert!(
            farm_under.staffing.iter().any(|row| row
                .professions
                .iter()
                .any(|gap| gap.name == "machinists" && gap.missing_here > 0.0)),
            "workshop should show how many more machinists it needs"
        );
        let needs = result
            .alerts
            .iter()
            .find(|alert| alert.kind == AlertKind::NeedsUnmet)
            .expect("needs");
        assert_eq!(needs.title, "Unmet pop needs in Test");
        let access = result
            .alerts
            .iter()
            .find(|alert| alert.kind == AlertKind::LowMarketAccess)
            .expect("access");
        assert_eq!(access.title, "Low market access in Test");
        let edu = result
            .alerts
            .iter()
            .find(|alert| alert.kind == AlertKind::UnfilledEducation)
            .expect("education");
        assert!(
            edu.title.contains("Test") && !edu.title.contains("state 1"),
            "qualification title should name the state, got {}",
            edu.title
        );
        assert!(
            edu.summary.to_ascii_lowercase().contains("jobs")
                && edu.summary.to_ascii_lowercase().contains("people"),
            "qualification summary should say jobs vs people, got {}",
            edu.summary
        );
    }

    #[test]
    fn fully_staffed_building_is_not_listed_as_underemployed() {
        let (world, defs, mut prices) = fixture();
        for building in &mut prices.buildings {
            building.staffing = building.level;
        }
        let result = alerts(&world, &defs, &prices);
        assert!(result
            .alerts
            .iter()
            .all(|alert| alert.kind != AlertKind::Underemployed
                && alert.kind != AlertKind::UnfilledPops));
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
                alert.kind == AlertKind::GoodsShortage
                    && alert.good_name.as_deref() == Some("grain")
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

    fn needs_unmet(world: &World, defs: &GameDefs, prices: &PricesResult) -> Alert {
        alerts(world, defs, prices)
            .alerts
            .into_iter()
            .find(|alert| alert.kind == AlertKind::NeedsUnmet)
            .expect("needs_unmet")
    }

    fn name_clothes(prices: &mut PricesResult, name: &str) {
        if let Some(good) = prices.goods.iter_mut().find(|good| good.name == "clothes") {
            good.label = Some(name.into());
        }
    }

    #[test]
    fn needs_unmet_evidence_prefers_amount_vs_sell_over_package() {
        let (world, defs, mut prices) = fixture();
        name_clothes(&mut prices, "Clothes");
        let result = alerts(&world, &defs, &prices);
        let needs = result
            .alerts
            .iter()
            .find(|alert| alert.kind == AlertKind::NeedsUnmet)
            .expect("needs");
        assert!(
            needs
                .evidence
                .iter()
                .all(|row| !row.value.contains("package") && !row.label.contains("package")),
            "package budget must not leak into evidence, got {:?}",
            needs.evidence
        );
        assert_eq!(needs.evidence.len(), 1);
        assert_eq!(needs.evidence[0].label, "Clothing: Clothes");
        assert_eq!(needs.evidence[0].value, "12 vs sell 0");
        assert_eq!(
            needs.summary,
            "Need goods exceed local sell, or sit at the max price."
        );
        assert!(result.limitations.iter().any(|line| {
            line.contains("need Amounts to local Sell")
                && line.contains("max price")
                && !line.contains("PRICE_RANGE")
                && !line.to_ascii_lowercase().contains("package")
        }));
        assert!(needs
            .mitigations
            .iter()
            .any(|mit| mit.title == "Cheapen Clothes"));
        assert!(needs
            .mitigations
            .iter()
            .any(|mit| mit.title == "Import Clothes through a trade center"));
    }

    #[test]
    fn needs_unmet_evidence_shows_price_over_base_at_max() {
        let (world, defs, mut prices) = fixture();
        name_clothes(&mut prices, "Clothes");
        let local = prices
            .state_goods
            .iter_mut()
            .find(|row| row.name == "clothes")
            .expect("clothes state good");
        local.sell = 20.0;
        local.price = 52.5;
        local.base = 30.0;
        let needs = needs_unmet(&world, &defs, &prices);
        assert_eq!(needs.evidence.len(), 1);
        assert_eq!(needs.evidence[0].label, "Clothing: Clothes");
        assert_eq!(needs.evidence[0].value, "52.50 / 30 (max)");
    }

    #[test]
    fn alerts_without_mitigations_leave_lists_empty() {
        let (world, defs, prices) = fixture();
        let result = alerts_with(
            &world,
            &defs,
            &prices,
            AlertsOptions {
                with_mitigations: false,
                mitigation_ids: None,
            },
        );
        assert!(!result.alerts.is_empty());
        assert!(result
            .alerts
            .iter()
            .all(|alert| alert.mitigations.is_empty()));
        assert!(kinds(&result).contains(&AlertKind::UnfilledEducation));
    }

    #[test]
    fn mitigation_ids_limit_which_alerts_get_lists() {
        let (world, defs, prices) = fixture();
        let lean = alerts_with(
            &world,
            &defs,
            &prices,
            AlertsOptions {
                with_mitigations: false,
                mitigation_ids: None,
            },
        );
        let Some(target) = lean.alerts.first().map(|a| a.id.clone()) else {
            return;
        };
        let mut ids = BTreeSet::new();
        ids.insert(target.clone());
        let result = alerts_with(
            &world,
            &defs,
            &prices,
            AlertsOptions {
                with_mitigations: true,
                mitigation_ids: Some(ids),
            },
        );
        let mut saw_target = false;
        for alert in &result.alerts {
            if alert.id == target {
                saw_target = true;
                // Fixture shortages / education usually have at least one lever.
                // Empty is allowed for kinds with no ranked advice.
            } else {
                assert!(
                    alert.mitigations.is_empty(),
                    "unexpected mitigations on {}",
                    alert.id
                );
            }
        }
        assert!(saw_target);
        let _ = result;
    }

    #[test]
    fn goods_shortage_alerts_skip_education_and_pops() {
        let (world, defs, prices) = fixture();
        let result = goods_shortage_alerts(&world, &defs, &prices, AlertsOptions::default());
        let kinds = kinds(&result);
        assert!(
            kinds.contains(&AlertKind::GoodsShortage)
                || kinds.contains(&AlertKind::ElectricityShortage)
                || kinds.contains(&AlertKind::TransportationShortage)
        );
        assert!(!kinds.contains(&AlertKind::UnfilledEducation));
        assert!(!kinds.contains(&AlertKind::UnfilledPops));
        assert!(!kinds.contains(&AlertKind::Underemployed));
        assert!(!kinds.contains(&AlertKind::NeedsUnmet));
        assert!(!kinds.contains(&AlertKind::LowMarketAccess));
    }

    /// Naive full-world scan (pre-index semantics) for equivalence checks.
    fn naive_type_pm_candidates(world: &World, defs: &GameDefs, type_id: &str) -> Vec<String> {
        let mut ids = BTreeSet::new();
        for building in &world.buildings {
            if building.type_script_id(defs) == type_id {
                ids.extend(building.production_methods.iter().cloned());
            }
        }
        ids.into_iter().collect()
    }

    fn naive_best_pm_upgrade(
        world: &World,
        defs: &GameDefs,
        good_name: &str,
        state_id: Option<u32>,
    ) -> Option<PmPick> {
        let idx = defs.index_of(good_name)?;
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
            let candidates = naive_type_pm_candidates(world, defs, building.type_script_id(defs));
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
                            building_type_name: building.type_script_id(defs).to_string(),
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

    fn naive_local_producer_type(
        world: &World,
        defs: &GameDefs,
        prices: &PricesResult,
        good_name: &str,
        state_id: Option<u32>,
    ) -> Option<String> {
        let producer = prices.buildings.iter().find(|building| {
            state_id.is_none_or(|sid| building.state_id == Some(sid))
                && building.outputs.iter().any(|flow| flow.name == good_name)
        });
        if let Some(row) = producer {
            return Some(row.building_type_name.clone());
        }
        world.buildings.iter().find_map(|row| {
            if state_id.is_some_and(|sid| row.state != Some(sid)) {
                return None;
            }
            let idx = defs.index_of(good_name)?;
            let (inputs, outputs) = row.goods_io(defs);
            (outputs[idx] > ORDER_EPS || inputs[idx] > ORDER_EPS)
                .then(|| row.type_script_id(defs).to_string())
        })
    }

    #[test]
    fn mitigation_index_matches_naive_pm_and_producer_picks() {
        let (world, defs, prices) = fixture();
        let index = MitigationIndex::build(&world, &defs, &prices);

        assert_eq!(
            index.buildings_of_type_in_state("building_rye_farm", Some(1)),
            vec![2, 4],
            "rye farms in state 1 are world indices 2 and 4"
        );
        assert_eq!(
            index.type_pm_candidates("building_rye_farm"),
            naive_type_pm_candidates(&world, &defs, "building_rye_farm")
        );
        assert_eq!(
            index.type_pm_candidates("building_rye_farm"),
            vec![
                "pm_simple_farming".to_string(),
                "pm_soil_enriching_farming".to_string(),
            ]
        );

        for state_id in [None, Some(1), Some(99)] {
            let indexed = index.best_pm_upgrade("grain", state_id);
            let naive = naive_best_pm_upgrade(&world, &defs, "grain", state_id);
            assert_eq!(indexed, naive, "best PM for grain state={state_id:?}");
            assert_eq!(
                index.local_producer_type("grain", state_id),
                naive_local_producer_type(&world, &defs, &prices, "grain", state_id),
                "local producer for grain state={state_id:?}"
            );
        }

        let pick = index
            .best_pm_upgrade("grain", Some(1))
            .expect("grain PM upgrade");
        assert_eq!(pick.building_id, 3);
        assert_eq!(pick.new_pm, "pm_soil_enriching_farming");
        assert_eq!(
            index.local_producer_type("grain", Some(1)).as_deref(),
            Some("building_rye_farm")
        );
        assert!(
            index.best_pm_upgrade("electricity", Some(1)).is_none(),
            "electricity has no PM upgrade on this fixture"
        );
    }
}
