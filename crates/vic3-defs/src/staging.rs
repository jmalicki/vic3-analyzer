//! Clausewitz load accumulates string good ids; resolve them to [`GoodIdx`] once.

use std::collections::{BTreeMap, HashMap};

use crate::{
    BuildingGroup, BuildingType, BuyPackage, DefsError, FlagDefinition, GameDefs, Good, GoodIdx,
    NeedEntry, PopNeed, ProductionMethod,
};

#[derive(Debug)]
pub(crate) struct StagingDefs {
    pub price_range: f64,
    pub goods_order: Vec<String>,
    pub goods: BTreeMap<String, Good>,
    pub labels: BTreeMap<String, String>,
    pub icons: BTreeMap<String, Vec<u8>>,
    pub flags: BTreeMap<String, Vec<u8>>,
    pub flag_defs: BTreeMap<String, Vec<FlagDefinition>>,
    pub production_methods: BTreeMap<String, StagingPm>,
    pub buildings: BTreeMap<String, BuildingType>,
    pub building_groups: BTreeMap<String, BuildingGroup>,
    pub pop_needs: BTreeMap<String, StagingNeed>,
    pub buy_packages: BTreeMap<u8, BuyPackage>,
    pub obsessions: BTreeMap<String, Vec<String>>,
}

impl Default for StagingDefs {
    fn default() -> Self {
        Self {
            price_range: crate::DEFAULT_PRICE_RANGE,
            goods_order: Vec::new(),
            goods: BTreeMap::new(),
            labels: BTreeMap::new(),
            icons: BTreeMap::new(),
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

impl StagingDefs {
    pub(crate) fn resolve(self) -> Result<GameDefs, DefsError> {
        let mut defs = GameDefs {
            price_range: self.price_range,
            goods_order: self.goods_order,
            goods: self.goods,
            labels: self.labels,
            icons: self.icons,
            flags: self.flags,
            flag_defs: self.flag_defs,
            production_methods: BTreeMap::new(),
            buildings: self.buildings,
            building_groups: self.building_groups,
            pop_needs: BTreeMap::new(),
            buy_packages: self.buy_packages,
            obsessions: BTreeMap::new(),
        };
        let index: HashMap<String, GoodIdx> = defs
            .goods_order
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), GoodIdx::from_usize(i)))
            .collect();
        let lookup = |name: &str| index.get(name).copied();

        for (id, pm) in self.production_methods {
            let inputs = pm
                .inputs
                .into_iter()
                .filter_map(|(good, qty)| Some((lookup(&good)?, qty)))
                .collect();
            let outputs = pm
                .outputs
                .into_iter()
                .filter_map(|(good, qty)| Some((lookup(&good)?, qty)))
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

        for (id, need) in self.pop_needs {
            let entries = need
                .entries
                .into_iter()
                .filter_map(|entry| {
                    Some(NeedEntry {
                        good: lookup(&entry.good)?,
                        weight: entry.weight,
                        min_supply_share: entry.min_supply_share,
                        max_supply_share: entry.max_supply_share,
                    })
                })
                .collect();
            let default_good = need.default_good.as_deref().and_then(lookup);
            defs.pop_needs.insert(
                id,
                PopNeed {
                    id: need.id,
                    default_good,
                    entries,
                },
            );
        }

        for (culture, goods) in self.obsessions {
            let idxs = goods
                .into_iter()
                .filter_map(|good| lookup(&good))
                .collect::<Vec<_>>();
            if !idxs.is_empty() {
                defs.obsessions.insert(culture, idxs);
            }
        }

        Ok(defs)
    }
}
