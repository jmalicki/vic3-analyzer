//! Clausewitz load accumulates string good/need ids; resolve them once.

use std::collections::{BTreeMap, HashMap};

use crate::types::build_package_ladder;
use crate::{
    BuildingGroup, BuildingType, BuyPackage, DefsError, FlagDefinition, GameDefs, Good, GoodIdx,
    NeedEntry, NeedIdx, NeedsVec, PopNeed, ProductionMethod,
};

#[derive(Debug, Clone)]
pub(crate) struct StagingDefs {
    pub price_range: f64,
    pub goods_order: Vec<String>,
    pub needs_order: Vec<String>,
    pub goods: BTreeMap<String, Good>,
    pub labels: BTreeMap<String, String>,
    pub icons: BTreeMap<String, Vec<u8>>,
    pub extra_icons: BTreeMap<String, Vec<u8>>,
    pub flags: BTreeMap<String, Vec<u8>>,
    pub flag_defs: BTreeMap<String, Vec<FlagDefinition>>,
    pub production_methods: BTreeMap<String, StagingPm>,
    pub buildings: BTreeMap<String, BuildingType>,
    pub building_groups: BTreeMap<String, BuildingGroup>,
    pub pop_needs: BTreeMap<String, StagingNeed>,
    pub buy_packages: BTreeMap<u8, StagingBuyPackage>,
    pub obsessions: BTreeMap<String, Vec<String>>,
}

impl Default for StagingDefs {
    fn default() -> Self {
        Self {
            price_range: crate::DEFAULT_PRICE_RANGE,
            goods_order: Vec::new(),
            needs_order: Vec::new(),
            goods: BTreeMap::new(),
            labels: BTreeMap::new(),
            icons: BTreeMap::new(),
            extra_icons: BTreeMap::new(),
            flags: BTreeMap::new(),
            flag_defs: BTreeMap::new(),
            production_methods: BTreeMap::new(),
            buildings: BTreeMap::new(),
            building_groups: BTreeMap::new(),
            pop_needs: BTreeMap::new(),
            buy_packages: BTreeMap::new(),
            obsessions: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct StagingPm {
    pub id: String,
    pub texture: Option<String>,
    pub inputs: BTreeMap<String, f64>,
    pub outputs: BTreeMap<String, f64>,
}

#[derive(Debug, Clone)]
pub(crate) struct StagingNeed {
    pub id: String,
    pub default_good: Option<String>,
    pub entries: Vec<StagingNeedEntry>,
}

#[derive(Debug, Clone)]
pub(crate) struct StagingNeedEntry {
    pub good: String,
    pub weight: f64,
    pub min_supply_share: f64,
    pub max_supply_share: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct StagingBuyPackage {
    pub wealth: u8,
    pub political_strength: f64,
    pub needs: BTreeMap<String, f64>,
}

impl StagingDefs {
    pub(crate) fn resolve(self) -> Result<GameDefs, DefsError> {
        let mut needs_order = self.needs_order;
        if needs_order.is_empty() && !self.pop_needs.is_empty() {
            needs_order = self.pop_needs.keys().cloned().collect();
        }
        for id in self.pop_needs.keys() {
            if !needs_order.contains(id) {
                needs_order.push(id.clone());
            }
        }

        let mut defs = GameDefs {
            price_range: self.price_range,
            goods_order: self.goods_order,
            needs_order,
            goods: self.goods,
            labels: self.labels,
            icons: self.icons,
            extra_icons: self.extra_icons,
            flags: self.flags,
            flag_defs: self.flag_defs,
            production_methods: BTreeMap::new(),
            buildings: self.buildings,
            building_groups: self.building_groups,
            pop_needs: Vec::new(),
            buy_packages: BTreeMap::new(),
            package_ladder: Vec::new(),
            obsessions: BTreeMap::new(),
        };
        crate::loc::polish_labels(&mut defs.labels);
        let good_index: HashMap<String, GoodIdx> = defs
            .goods_order
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), GoodIdx::from_usize(i)))
            .collect();
        let need_index: HashMap<String, NeedIdx> = defs
            .needs_order
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), NeedIdx::from_usize(i)))
            .collect();
        let lookup_good = |name: &str| good_index.get(name).copied();
        let lookup_need = |name: &str| need_index.get(name).copied();

        for (id, pm) in self.production_methods {
            let inputs = pm
                .inputs
                .into_iter()
                .filter_map(|(good, qty)| Some((lookup_good(&good)?, qty)))
                .collect();
            let outputs = pm
                .outputs
                .into_iter()
                .filter_map(|(good, qty)| Some((lookup_good(&good)?, qty)))
                .collect();
            defs.production_methods.insert(
                id,
                ProductionMethod {
                    id: pm.id,
                    inputs,
                    outputs,
                },
            );
        }

        defs.pop_needs = vec![
            PopNeed {
                id: String::new(),
                default_good: None,
                entries: Vec::new(),
            };
            defs.needs_order.len()
        ];
        for (id, need) in self.pop_needs {
            let Some(idx) = lookup_need(&id) else {
                continue;
            };
            let entries = need
                .entries
                .into_iter()
                .filter_map(|entry| {
                    Some(NeedEntry {
                        good: lookup_good(&entry.good)?,
                        weight: entry.weight,
                        min_supply_share: entry.min_supply_share,
                        max_supply_share: entry.max_supply_share,
                    })
                })
                .collect();
            let default_good = need.default_good.as_deref().and_then(lookup_good);
            defs.pop_needs[idx.as_usize()] = PopNeed {
                id: need.id,
                default_good,
                entries,
            };
        }

        let n_needs = defs.needs_order.len();
        for (wealth, package) in self.buy_packages {
            let mut needs = NeedsVec::zeros(n_needs);
            for (need_id, value) in package.needs {
                if let Some(idx) = lookup_need(&need_id) {
                    needs[idx] = value;
                }
            }
            defs.buy_packages.insert(
                wealth,
                BuyPackage {
                    wealth: package.wealth,
                    political_strength: package.political_strength,
                    needs,
                },
            );
        }
        defs.package_ladder = build_package_ladder(&defs.buy_packages, n_needs);

        for (culture, goods) in self.obsessions {
            let idxs = goods
                .into_iter()
                .filter_map(|good| lookup_good(&good))
                .collect::<Vec<_>>();
            if !idxs.is_empty() {
                defs.obsessions.insert(culture, idxs);
            }
        }

        Ok(defs)
    }
}
