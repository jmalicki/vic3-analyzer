use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use jomini::text::ObjectReader;
use jomini::{Encoding, JominiDeserialize, TextTape};
use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::{
    BuyPackage, DefsError, GameDefs, Good, NeedEntry, PopNeed, ProductionMethod,
    DEFAULT_PRICE_RANGE,
};

/// Load definitions from a Victoria 3 install or a fixture tree.
///
/// # Expected layout
///
/// `root` may be either:
/// - a game install whose data lives under `game/` (Steam/PDX launcher layout)
/// - a fixture / unpacked `game` directory that already contains `common/`
///
/// Relative paths under the data root:
/// - `common/goods` — good id → `cost` (base price); `base_price` is also accepted
/// - `common/defines` — `NEconomy.PRICE_RANGE` (also `NDefines.NEconomy` or top-level)
/// - `common/production_methods` — PM ids plus `goods_input_*` / `goods_output_*`
/// - `common/pop_needs` — need substitution tables (`entry` / min & max supply share)
/// - `common/buy_packages` — `wealth_N` packages
/// - `common/cultures` — optional `obsessions = { good_id ... }` (empty is fine)
///
/// All `*.txt` files in those directories are merged in sorted path order; later
/// files override the same id.
pub fn load_from_path(root: impl AsRef<Path>) -> Result<GameDefs, DefsError> {
    let data_root = resolve_data_root(root.as_ref())?;
    let mut defs = GameDefs {
        price_range: load_price_range(&data_root)?,
        ..GameDefs::default()
    };
    defs.goods = load_goods(&data_root)?;
    defs.production_methods = load_production_methods(&data_root)?;
    defs.pop_needs = load_pop_needs(&data_root)?;
    defs.buy_packages = load_buy_packages(&data_root)?;
    defs.obsessions = load_obsessions(&data_root)?;
    Ok(defs)
}

fn resolve_data_root(root: &Path) -> Result<PathBuf, DefsError> {
    if root.join("common/goods").is_dir() {
        return Ok(root.to_path_buf());
    }
    let game = root.join("game");
    if game.join("common/goods").is_dir() {
        return Ok(game);
    }
    Err(DefsError::NotAGameRoot(root.to_path_buf()))
}

fn load_price_range(data_root: &Path) -> Result<f64, DefsError> {
    let mut price_range = DEFAULT_PRICE_RANGE;
    for path in txt_files(&data_root.join("common/defines"))? {
        let file: RawDefinesFile = parse_file(&path)?;
        if let Some(v) = file.price_range() {
            price_range = v;
        }
    }
    Ok(price_range)
}

fn load_goods(data_root: &Path) -> Result<BTreeMap<String, Good>, DefsError> {
    let mut goods = BTreeMap::new();
    for path in txt_files(&data_root.join("common/goods"))? {
        let file: BTreeMap<String, RawGood> = parse_file(&path)?;
        for (id, raw) in file {
            let Some(base_price) = raw.base_price() else {
                continue;
            };
            goods.insert(id.clone(), Good { id, base_price });
        }
    }
    Ok(goods)
}

fn load_production_methods(
    data_root: &Path,
) -> Result<BTreeMap<String, ProductionMethod>, DefsError> {
    let mut pms = BTreeMap::new();
    for path in txt_files(&data_root.join("common/production_methods"))? {
        for pm in parse_production_methods(&path)? {
            pms.insert(pm.id.clone(), pm);
        }
    }
    Ok(pms)
}

fn parse_production_methods(path: &Path) -> Result<Vec<ProductionMethod>, DefsError> {
    let bytes = std::fs::read(path).map_err(|source| DefsError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let bytes = strip_bom(&bytes);
    if looks_empty(bytes) {
        return Ok(Vec::new());
    }
    let tape = TextTape::from_slice(bytes).map_err(|source| DefsError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    let reader = tape.utf8_reader();
    let mut out = Vec::new();
    for (key, _op, value) in reader.fields() {
        let id = key.read_str().to_string();
        let mut inputs = BTreeMap::new();
        let mut outputs = BTreeMap::new();
        if let Ok(obj) = value.read_object() {
            collect_goods_modifiers(&obj, &mut inputs, &mut outputs);
        }
        out.push(ProductionMethod {
            id,
            inputs,
            outputs,
        });
    }
    Ok(out)
}

fn collect_goods_modifiers<E: Encoding + Clone>(
    obj: &ObjectReader<'_, '_, E>,
    inputs: &mut BTreeMap<String, f64>,
    outputs: &mut BTreeMap<String, f64>,
) {
    for (key, _op, value) in obj.fields() {
        let name = key.read_str();
        if let Some((good, is_output)) = goods_modifier_key(name.as_ref()) {
            if let Some(num) = value.read_scalar().ok().and_then(|s| s.to_f64().ok()) {
                let dest = if is_output {
                    &mut *outputs
                } else {
                    &mut *inputs
                };
                *dest.entry(good.to_string()).or_insert(0.0) += num;
            }
            continue;
        }
        if let Ok(nested) = value.read_object() {
            collect_goods_modifiers(&nested, inputs, outputs);
        }
    }
}

fn load_pop_needs(data_root: &Path) -> Result<BTreeMap<String, PopNeed>, DefsError> {
    let mut needs = BTreeMap::new();
    for path in txt_files(&data_root.join("common/pop_needs"))? {
        let file: BTreeMap<String, RawNeed> = parse_file(&path)?;
        for (id, raw) in file {
            let entries = raw
                .entry
                .into_iter()
                .filter_map(|e| {
                    e.goods.map(|good| NeedEntry {
                        good,
                        weight: e.weight.unwrap_or(1.0),
                        min_supply_share: e.min_supply_share.unwrap_or(0.0),
                        max_supply_share: e.max_supply_share.unwrap_or(1.0),
                    })
                })
                .collect();
            needs.insert(
                id.clone(),
                PopNeed {
                    id,
                    default_good: raw.default,
                    entries,
                },
            );
        }
    }
    Ok(needs)
}

fn load_buy_packages(data_root: &Path) -> Result<BTreeMap<u8, BuyPackage>, DefsError> {
    let mut packages = BTreeMap::new();
    for path in txt_files(&data_root.join("common/buy_packages"))? {
        let file: BTreeMap<String, RawBuyPackage> = parse_file(&path)?;
        for (key, raw) in file {
            let Some(wealth) = parse_wealth_key(&key) else {
                continue;
            };
            packages.insert(
                wealth,
                BuyPackage {
                    wealth,
                    political_strength: raw.political_strength.unwrap_or(0.0),
                    needs: raw.goods,
                },
            );
        }
    }
    Ok(packages)
}

fn load_obsessions(data_root: &Path) -> Result<BTreeMap<String, Vec<String>>, DefsError> {
    let mut obsessions = BTreeMap::new();
    for path in txt_files(&data_root.join("common/cultures"))? {
        let file: BTreeMap<String, RawCulture> = parse_file(&path)?;
        for (culture, raw) in file {
            if !raw.obsessions.is_empty() {
                obsessions.insert(culture, raw.obsessions);
            }
        }
    }
    Ok(obsessions)
}

fn parse_wealth_key(key: &str) -> Option<u8> {
    key.strip_prefix("wealth_")?.parse().ok()
}

fn txt_files(dir: &Path) -> Result<Vec<PathBuf>, DefsError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    collect_txt(dir, &mut files).map_err(|source| DefsError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    files.sort();
    Ok(files)
}

fn collect_txt(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_txt(&path, files)?;
        } else if path.extension().is_some_and(|ext| ext == "txt") {
            files.push(path);
        }
    }
    Ok(())
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

fn looks_empty(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(|s| {
        s.lines().all(|line| {
            let trimmed = line.trim();
            trimmed.is_empty() || trimmed.starts_with('#')
        })
    })
}

fn parse_file<T>(path: &Path) -> Result<T, DefsError>
where
    T: DeserializeOwned + Default,
{
    let bytes = std::fs::read(path).map_err(|source| DefsError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let bytes = strip_bom(&bytes);
    if looks_empty(bytes) {
        return Ok(T::default());
    }
    jomini::text::de::from_utf8_slice(bytes)
        .or_else(|_| jomini::text::de::from_windows1252_slice(bytes))
        .map_err(|source| DefsError::Parse {
            path: path.to_path_buf(),
            source,
        })
}

#[derive(Debug, Default, Deserialize)]
struct RawDefinesFile {
    #[serde(rename = "PRICE_RANGE")]
    price_range: Option<f64>,
    #[serde(rename = "NEconomy")]
    n_economy: Option<RawNEconomy>,
    #[serde(rename = "NDefines")]
    n_defines: Option<RawNDefines>,
}

impl RawDefinesFile {
    fn price_range(&self) -> Option<f64> {
        self.n_economy
            .as_ref()
            .and_then(|e| e.price_range)
            .or_else(|| {
                self.n_defines
                    .as_ref()
                    .and_then(|d| d.n_economy.as_ref().and_then(|e| e.price_range))
            })
            .or(self.price_range)
    }
}

#[derive(Debug, Default, Deserialize)]
struct RawNDefines {
    #[serde(rename = "NEconomy")]
    n_economy: Option<RawNEconomy>,
}

#[derive(Debug, Default, Deserialize)]
struct RawNEconomy {
    #[serde(rename = "PRICE_RANGE")]
    price_range: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct RawGood {
    cost: Option<f64>,
    base_price: Option<f64>,
}

impl RawGood {
    fn base_price(&self) -> Option<f64> {
        self.cost.or(self.base_price)
    }
}

fn goods_modifier_key(key: &str) -> Option<(&str, bool)> {
    let (rest, is_output) = if let Some(rest) = key.strip_prefix("goods_output_") {
        (rest, true)
    } else if let Some(rest) = key.strip_prefix("goods_input_") {
        (rest, false)
    } else {
        return None;
    };
    let good = rest.strip_suffix("_add").unwrap_or(rest);
    if good.is_empty() {
        None
    } else {
        Some((good, is_output))
    }
}

#[derive(Debug, Default, JominiDeserialize)]
struct RawNeed {
    default: Option<String>,
    #[jomini(duplicated, alias = "entry")]
    entry: Vec<RawNeedEntry>,
}

#[derive(Debug, Default, JominiDeserialize)]
struct RawNeedEntry {
    goods: Option<String>,
    weight: Option<f64>,
    min_supply_share: Option<f64>,
    max_supply_share: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct RawBuyPackage {
    political_strength: Option<f64>,
    #[serde(default)]
    goods: BTreeMap<String, f64>,
}

#[derive(Debug, Default, Deserialize)]
struct RawCulture {
    #[serde(default)]
    obsessions: Vec<String>,
}
