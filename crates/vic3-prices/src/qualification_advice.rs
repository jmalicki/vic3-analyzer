//! Qualification-shortage levers: game-data graph + YAML copy.
//!
//! Relevance comes from `pop_types` (or a bundled vanilla snapshot) and the
//! state's current mix. Wording is in `advice/qualifications/*.yml`. Used by
//! [`crate::alerts`] employment expanders ([`BuildingStaffing`], profession
//! gaps) — not a monthly qualification-rate simulator.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use serde::Deserialize;
use vic3_defs::{GameDefs, PopType, QualificationFactors};

use crate::result::{BuildingEconomics, ProfessionCount};
use crate::{PricesResult, ORDER_EPS};

pub(crate) const LITERACY_UNIVERSITY: f64 = 0.5;
const SOURCE_PRESENT: f64 = 1.0;
const MAX_FEEDERS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum LeverKey {
    Farm,
    Mine,
    Workshop,
    Government,
    Urban,
    Barracks,
    University,
    Wealth,
    AlwaysHire,
}

impl LeverKey {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Farm => "farm",
            Self::Mine => "mine",
            Self::Workshop => "workshop",
            Self::Government => "government",
            Self::Urban => "urban",
            Self::Barracks => "barracks",
            Self::University => "university",
            Self::Wealth => "wealth",
            Self::AlwaysHire => "always_hire",
        }
    }

    fn employs_target(self, target: &str) -> bool {
        match self {
            Self::Workshop => id_has(target, "machinist"),
            Self::University => id_has(target, "academic"),
            Self::Government => id_has(target, "bureaucrat") || id_has(target, "clerk"),
            Self::Urban => id_has(target, "shopkeeper") || id_has(target, "clerg"),
            Self::Barracks => id_has(target, "soldier") || id_has(target, "officer"),
            Self::Farm => id_has(target, "farmer"),
            Self::Mine | Self::Wealth | Self::AlwaysHire => false,
        }
    }

    pub(crate) fn fallback_building_owned(self) -> String {
        self.fallback_building().to_string()
    }

    pub(crate) fn fallback_building(self) -> &'static str {
        match self {
            Self::Farm => "building_rye_farm",
            Self::Mine => "building_coal_mine",
            Self::Workshop => "building_tooling_workshop",
            Self::Government => "building_government_administration",
            Self::Urban => "building_urban_center",
            Self::Barracks => "building_barracks",
            Self::University => "building_university",
            Self::Wealth | Self::AlwaysHire => "",
        }
    }

    fn needles(self) -> &'static [&'static str] {
        match self {
            Self::Farm => &["farm"],
            Self::Mine => &["mine"],
            Self::Workshop => &["workshop", "manufactur"],
            Self::Government => &["government"],
            Self::Urban => &["urban_center"],
            Self::Barracks => &["barrack", "headquarter"],
            Self::University => &["university"],
            Self::Wealth | Self::AlwaysHire => &[],
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ChosenLever {
    pub key: LeverKey,
    pub title: String,
    pub detail: String,
    pub building: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct StateMix {
    pub by_profession: BTreeMap<String, f64>,
    pub literacy: f64,
}

impl StateMix {
    pub(crate) fn count(&self, profession: &str) -> f64 {
        self.by_profession
            .iter()
            .filter(|(id, _)| id_has(id, profession) || id.as_str() == profession)
            .map(|(_, n)| *n)
            .sum()
    }

    pub(crate) fn peasants(&self) -> f64 {
        self.count("peasant")
    }

    pub(crate) fn summary(&self) -> String {
        let mut parts: Vec<(String, f64)> = self
            .by_profession
            .iter()
            .map(|(id, n)| (id.clone(), *n))
            .collect();
        parts.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        parts.truncate(4);
        if parts.is_empty() {
            return "no workforce counted".into();
        }
        parts
            .into_iter()
            .map(|(id, n)| format!("{id} {}", format_pop(n)))
            .collect::<Vec<_>>()
            .join(" / ")
    }

    fn abundant(&self) -> Vec<String> {
        let total: f64 = self.by_profession.values().sum();
        let floor = (total * 0.05).max(SOURCE_PRESENT);
        self.by_profession
            .iter()
            .filter(|(_, n)| **n >= floor)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

pub(crate) fn state_mix(prices: &PricesResult, state_id: u32) -> StateMix {
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
        if let Some(prof) = pop.profession_name.as_deref() {
            *mix.by_profession.entry(prof.to_string()).or_insert(0.0) += n;
        }
    }
    mix.literacy = if size > 0.0 { literate / size } else { 0.0 };
    mix
}

fn pop_type_for(defs: &GameDefs, target: &str) -> PopType {
    if let Some(existing) = defs.pop_types.get(target).cloned().or_else(|| {
        defs.pop_types
            .iter()
            .find_map(|(id, pop)| id_has(id, target).then_some(pop.clone()))
    }) {
        return existing;
    }
    vanilla_graph()
        .get(target)
        .cloned()
        .or_else(|| {
            vanilla_graph()
                .iter()
                .find_map(|(id, pop)| id_has(id, target).then_some(pop.clone()))
        })
        .unwrap_or(PopType {
            id: target.to_string(),
            ..PopType::default()
        })
}

pub(crate) fn select_levers(
    prices: &PricesResult,
    defs: &GameDefs,
    state_id: u32,
    target: &str,
    mix: &StateMix,
) -> Vec<ChosenLever> {
    let pop = pop_type_for(defs, target);
    let copy = profession_copy(target);
    if pop.can_always_hire {
        return vec![fill_lever(
            LeverKey::AlwaysHire,
            &copy,
            target,
            target,
            None,
        )];
    }

    let factors = &pop.qualifications;
    let sources = source_list(factors);
    let present: Vec<&str> = sources
        .iter()
        .filter(|source| mix.count(source) >= SOURCE_PRESENT)
        .map(String::as_str)
        .collect();
    let missing: Vec<&str> = sources
        .iter()
        .filter(|source| mix.count(source) < SOURCE_PRESENT)
        .map(String::as_str)
        .collect();
    let mut out = Vec::new();

    let abundant = mix.abundant();
    let mut feeders = Vec::new();
    for source in &missing {
        for key in levers_for_source(source) {
            if key.employs_target(target) {
                continue;
            }
            let hireable = can_hire_from(&abundant, source) || mix.peasants() >= SOURCE_PRESENT;
            if !hireable && mix.peasants() < SOURCE_PRESENT {
                continue;
            }
            if feeders.iter().any(|(existing, _, _, _)| *existing == key) {
                continue;
            }
            let weight = factors
                .source_multipliers
                .get(*source)
                .copied()
                .unwrap_or(1.0);
            let ready = if can_hire_from(&abundant, source) {
                1
            } else {
                0
            };
            feeders.push((key, (*source).to_string(), ready, weight));
        }
    }
    feeders.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then(b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal))
    });
    feeders.truncate(MAX_FEEDERS);
    for (key, source, _, _) in feeders {
        let building = building_for_lever(prices, defs, state_id, key);
        out.push(fill_lever(key, &copy, target, &source, building));
    }

    if !present.is_empty() {
        if factors.wealth {
            out.push(fill_lever(
                LeverKey::Wealth,
                &copy,
                target,
                present[0],
                None,
            ));
        }
        if factors.literacy && mix.literacy + ORDER_EPS >= LITERACY_UNIVERSITY {
            let uni = building_for_lever(prices, defs, state_id, LeverKey::University);
            out.push(fill_lever(
                LeverKey::University,
                &copy,
                target,
                present[0],
                uni,
            ));
        }
    }
    out
}

fn source_list(factors: &QualificationFactors) -> Vec<String> {
    let mut rows: Vec<(String, f64)> = factors
        .source_multipliers
        .iter()
        .map(|(id, weight)| (id.clone(), *weight))
        .collect();
    rows.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    rows.into_iter().map(|(id, _)| id).collect()
}

fn levers_for_source(source: &str) -> Vec<LeverKey> {
    let mut keys = Vec::new();
    if id_has(source, "farmer") || id_has(source, "laborer") || id_has(source, "peasant") {
        keys.push(LeverKey::Farm);
    }
    if id_has(source, "laborer") || id_has(source, "miner") {
        keys.push(LeverKey::Mine);
    }
    if id_has(source, "machinist") {
        keys.push(LeverKey::Workshop);
    }
    if id_has(source, "bureaucrat") || id_has(source, "clerk") {
        keys.push(LeverKey::Government);
    }
    if id_has(source, "shopkeeper") || id_has(source, "clerg") {
        keys.push(LeverKey::Urban);
    }
    if id_has(source, "soldier") || id_has(source, "officer") {
        keys.push(LeverKey::Barracks);
    }
    keys
}

fn can_hire_from(abundant: &[String], source: &str) -> bool {
    if id_has(source, "farmer") || id_has(source, "laborer") {
        return abundant
            .iter()
            .any(|id| id_has(id, "peasant") || id_has(id, "laborer"));
    }
    abundant.iter().any(|id| {
        id_has(id, source)
            || (id_has(source, "clerk") && (id_has(id, "laborer") || id_has(id, "machinist")))
            || (id_has(source, "bureaucrat") && id_has(id, "clerk"))
            || (id_has(source, "machinist") && id_has(id, "laborer"))
    })
}

fn fill_lever(
    key: LeverKey,
    copy: &ProfessionCopy,
    target: &str,
    source: &str,
    building: Option<String>,
) -> ChosenLever {
    let text = copy.levers.get(key.as_str()).cloned().unwrap_or_else(|| {
        defaults()
            .levers
            .get(key.as_str())
            .cloned()
            .unwrap_or(LeverCopy {
                title: key.as_str().into(),
                detail: String::new(),
            })
    });
    let building_name = building.as_deref().unwrap_or(key.fallback_building());
    ChosenLever {
        key,
        title: subst(&text.title, target, source, building_name),
        detail: subst(&text.detail, target, source, building_name),
        building,
        source: source.to_string(),
    }
}

fn subst(text: &str, target: &str, source: &str, building: &str) -> String {
    text.replace("{target}", target)
        .replace("{source}", source)
        .replace("{building}", building)
}

fn building_for_lever(
    prices: &PricesResult,
    defs: &GameDefs,
    state_id: u32,
    key: LeverKey,
) -> Option<String> {
    let from_state = prices.buildings.iter().find_map(|building| {
        if building.state_id != Some(state_id) {
            return None;
        }
        if is_subsistence(&building.type_id, defs) {
            return None;
        }
        lever_matches_building(key, &building.type_id, defs).then(|| building.type_id.clone())
    });
    from_state.or_else(|| {
        defs.buildings.keys().find_map(|id| {
            if is_subsistence(id, defs) {
                return None;
            }
            lever_matches_building(key, id, defs).then(|| id.clone())
        })
    })
}

fn lever_matches_building(key: LeverKey, type_id: &str, defs: &GameDefs) -> bool {
    if key.needles().iter().any(|needle| id_has(type_id, needle)) {
        if key == LeverKey::Farm && is_subsistence(type_id, defs) {
            return false;
        }
        return true;
    }
    let Some(building) = defs.buildings.get(type_id) else {
        return false;
    };
    if key == LeverKey::Farm
        && building.city_type.as_deref() == Some("farm")
        && !is_subsistence(type_id, defs)
    {
        return true;
    }
    professions_employed(defs, type_id)
        .iter()
        .any(|prof| levers_for_source(prof).contains(&key) && !key.employs_target(prof))
}

pub(crate) fn is_subsistence(type_id: &str, defs: &GameDefs) -> bool {
    if id_has(type_id, "subsistence") {
        return true;
    }
    defs.buildings
        .get(type_id)
        .and_then(|building| building.group.as_deref())
        .is_some_and(|group| id_has(group, "subsistence"))
}

pub(crate) fn professions_employed(defs: &GameDefs, type_id: &str) -> Vec<String> {
    let Some(building) = defs.buildings.get(type_id) else {
        return Vec::new();
    };
    let mut counts: BTreeMap<String, f64> = BTreeMap::new();
    for group in &building.production_method_groups {
        let Some(pms) = defs.production_method_groups.get(group) else {
            continue;
        };
        for pm_id in pms {
            let Some(pm) = defs.production_methods.get(pm_id) else {
                continue;
            };
            for (prof, qty) in &pm.employment {
                *counts.entry(prof.clone()).or_insert(0.0) += *qty;
            }
        }
    }
    counts.into_keys().collect()
}

#[derive(Debug, Clone, Deserialize, Default)]
struct LeverCopy {
    title: String,
    #[serde(default)]
    detail: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ProfessionCopy {
    #[serde(default)]
    levers: BTreeMap<String, LeverCopy>,
}

#[derive(Debug, Deserialize)]
struct VanillaFile {
    professions: BTreeMap<String, VanillaPop>,
}

#[derive(Debug, Deserialize, Default)]
struct VanillaPop {
    #[serde(default)]
    can_always_hire: bool,
    #[serde(default)]
    literacy: bool,
    #[serde(default)]
    wealth: bool,
    #[serde(default)]
    wealth_floor: Option<f64>,
    #[serde(default)]
    sources: BTreeMap<String, f64>,
}

fn defaults() -> &'static ProfessionCopy {
    static COPY: OnceLock<ProfessionCopy> = OnceLock::new();
    COPY.get_or_init(|| {
        serde_yaml::from_str(include_str!("../advice/qualifications/_defaults.yml"))
            .expect("qualification advice _defaults.yml")
    })
}

fn profession_files() -> &'static HashMap<String, ProfessionCopy> {
    static FILES: OnceLock<HashMap<String, ProfessionCopy>> = OnceLock::new();
    FILES.get_or_init(|| {
        const RAW: &[(&str, &str)] = &[
            (
                "academics",
                include_str!("../advice/qualifications/academics.yml"),
            ),
            (
                "aristocrats",
                include_str!("../advice/qualifications/aristocrats.yml"),
            ),
            (
                "bureaucrats",
                include_str!("../advice/qualifications/bureaucrats.yml"),
            ),
            (
                "capitalists",
                include_str!("../advice/qualifications/capitalists.yml"),
            ),
            (
                "clergymen",
                include_str!("../advice/qualifications/clergymen.yml"),
            ),
            (
                "clerks",
                include_str!("../advice/qualifications/clerks.yml"),
            ),
            (
                "engineers",
                include_str!("../advice/qualifications/engineers.yml"),
            ),
            (
                "farmers",
                include_str!("../advice/qualifications/farmers.yml"),
            ),
            (
                "laborers",
                include_str!("../advice/qualifications/laborers.yml"),
            ),
            (
                "machinists",
                include_str!("../advice/qualifications/machinists.yml"),
            ),
            (
                "officers",
                include_str!("../advice/qualifications/officers.yml"),
            ),
            (
                "peasants",
                include_str!("../advice/qualifications/peasants.yml"),
            ),
            (
                "shopkeepers",
                include_str!("../advice/qualifications/shopkeepers.yml"),
            ),
            (
                "slaves",
                include_str!("../advice/qualifications/slaves.yml"),
            ),
            (
                "soldiers",
                include_str!("../advice/qualifications/soldiers.yml"),
            ),
        ];
        RAW.iter()
            .map(|(id, text)| {
                (
                    (*id).to_string(),
                    serde_yaml::from_str(text)
                        .unwrap_or_else(|err| panic!("qualification advice {id}.yml: {err}")),
                )
            })
            .collect()
    })
}

fn profession_copy(target: &str) -> ProfessionCopy {
    let files = profession_files();
    files
        .get(target)
        .cloned()
        .or_else(|| {
            files
                .iter()
                .find_map(|(id, copy)| id_has(target, id).then_some(copy.clone()))
        })
        .unwrap_or_else(|| defaults().clone())
}

fn vanilla_graph() -> &'static BTreeMap<String, PopType> {
    static GRAPH: OnceLock<BTreeMap<String, PopType>> = OnceLock::new();
    GRAPH.get_or_init(|| {
        let file: VanillaFile =
            serde_yaml::from_str(include_str!("../advice/qualifications/_vanilla_graph.yml"))
                .expect("qualification _vanilla_graph.yml");
        file.professions
            .into_iter()
            .map(|(id, raw)| {
                (
                    id.clone(),
                    PopType {
                        id,
                        can_always_hire: raw.can_always_hire,
                        qualifications: QualificationFactors {
                            literacy: raw.literacy,
                            wealth: raw.wealth,
                            wealth_floor: raw.wealth_floor,
                            source_multipliers: raw.sources,
                        },
                    },
                )
            })
            .collect()
    })
}

pub(crate) fn id_has(id: &str, needle: &str) -> bool {
    id.to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn format_pop(value: f64) -> String {
    if (value - value.round()).abs() < 1e-6 {
        format!("{:.0}", value.round())
    } else {
        format!("{value:.0}")
    }
}

/// Levels that still look fully staffed after display rounding (8.00 / 8 at 100%).
pub(crate) const STAFFED_LEVEL_EPS: f64 = 0.05;
pub(crate) const STAFFED_RATIO: f64 = 0.995;

pub(crate) fn is_fully_staffed(staffing: f64, level: f64) -> bool {
    if level <= ORDER_EPS {
        return true;
    }
    level - staffing < STAFFED_LEVEL_EPS || staffing / level >= STAFFED_RATIO
}

/// Per-profession shortfall on one building vs the whole state stock.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ProfessionGap {
    pub name: String,
    // FIXME: `label` should always be present (at least a humanized script key).
    // Optional/null is a leftover from the old dialect; fixing emitters is out of
    // scope for the name/label rename wave.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub employed_here: f64,
    pub jobs_here: f64,
    pub missing_here: f64,
    pub state_jobs: f64,
    pub state_stock: f64,
    pub state_shortage: f64,
}

/// Per-building staffing rows under a state-level employment alert.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct BuildingStaffing {
    pub building_id: u32,
    pub building_name: String,
    pub type_id: String,
    pub staffing: f64,
    pub level: f64,
    pub professions: Vec<ProfessionGap>,
}

pub(crate) fn building_staffing(
    defs: &GameDefs,
    prices: &PricesResult,
    building: &BuildingEconomics,
    name: String,
) -> BuildingStaffing {
    let ratio = if building.staffing > ORDER_EPS {
        building.level / building.staffing
    } else {
        0.0
    };
    let professions = building
        .employees
        .iter()
        .filter(|row| row.count > ORDER_EPS || ratio > 0.0)
        .map(|row| profession_gap(defs, prices, building.state_id, row, ratio))
        .collect();
    BuildingStaffing {
        building_id: building.id,
        building_name: name,
        type_id: building.type_id.clone(),
        staffing: building.staffing,
        level: building.level,
        professions,
    }
}

fn profession_gap(
    defs: &GameDefs,
    prices: &PricesResult,
    state_id: Option<u32>,
    row: &ProfessionCount,
    scale: f64,
) -> ProfessionGap {
    let jobs_here = if scale > 0.0 {
        row.count * scale
    } else {
        row.count
    };
    let missing_here = (jobs_here - row.count).max(0.0);
    let qual = state_id.and_then(|id| {
        prices
            .state_qualifications
            .iter()
            .find(|item| item.state_id == id && item.name == row.name)
    });
    ProfessionGap {
        name: row.name.clone(),
        label: row
            .label
            .clone()
            .or_else(|| defs.labels.get(&row.name).cloned()),
        employed_here: row.count,
        jobs_here,
        missing_here,
        state_jobs: qual.map(|row| row.jobs).unwrap_or(0.0),
        state_stock: qual
            .map(|row| row.employable.unwrap_or(row.qualified))
            .unwrap_or(0.0),
        state_shortage: qual.map(|row| row.shortage).unwrap_or(0.0),
    }
}

pub(crate) fn blocking_profession(staffing: &[BuildingStaffing]) -> Option<&ProfessionGap> {
    staffing
        .iter()
        .flat_map(|building| building.professions.iter())
        .filter(|row| row.missing_here > ORDER_EPS || row.state_shortage > ORDER_EPS)
        .max_by(|a, b| {
            match a
                .state_shortage
                .partial_cmp(&b.state_shortage)
                .unwrap_or(std::cmp::Ordering::Equal)
            {
                std::cmp::Ordering::Equal => a
                    .missing_here
                    .partial_cmp(&b.missing_here)
                    .unwrap_or(std::cmp::Ordering::Equal),
                other => other,
            }
        })
}

pub(crate) fn state_profession_totals(staffing: &[BuildingStaffing]) -> Vec<ProfessionGap> {
    let mut by_id: BTreeMap<String, ProfessionGap> = BTreeMap::new();
    for building in staffing {
        for row in &building.professions {
            let entry = by_id.entry(row.name.clone()).or_insert(ProfessionGap {
                name: row.name.clone(),
                label: row.label.clone(),
                employed_here: 0.0,
                jobs_here: 0.0,
                missing_here: 0.0,
                state_jobs: row.state_jobs,
                state_stock: row.state_stock,
                state_shortage: row.state_shortage,
            });
            entry.employed_here += row.employed_here;
            entry.jobs_here += row.jobs_here;
            entry.missing_here += row.missing_here;
        }
    }
    let mut rows: Vec<_> = by_id.into_values().collect();
    rows.sort_by(|a, b| {
        b.state_shortage
            .partial_cmp(&a.state_shortage)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                b.missing_here
                    .partial_cmp(&a.missing_here)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    rows
}

pub(crate) fn shortage_target<'a>(
    prices: &'a PricesResult,
    building: &BuildingEconomics,
) -> Option<&'a str> {
    let state_id = building.state_id?;
    building.employees.iter().find_map(|employee| {
        prices
            .state_qualifications
            .iter()
            .find(|row| {
                row.state_id == state_id && row.name == employee.name && row.shortage > ORDER_EPS
            })
            .map(|row| row.name.as_str())
    })
}
