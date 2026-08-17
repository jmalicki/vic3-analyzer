use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use jomini::text::ObjectReader;
use jomini::{Encoding, JominiDeserialize, TextTape};
use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::{
    classify_defs_path, icons,
    staging::{StagingBuyPackage, StagingDefs, StagingNeed, StagingNeedEntry, StagingPm},
    BuildingGroup, BuildingType, DefsError, DefsPathClass, GameDefs, Good, DEFAULT_PRICE_RANGE,
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
/// - `common/buildings` — building type → group / city type
/// - `common/building_groups` — category, land usage, and default building
/// - `common/pop_needs` — need substitution tables (`entry` / min & max supply share)
/// - `common/buy_packages` — `wealth_N` packages
/// - `common/cultures` — optional `obsessions = { good_id ... }` (empty is fine)
///
/// All `*.txt` files in those directories are merged in sorted path order; later
/// files override the same id.
pub fn load_from_path(root: impl AsRef<Path>) -> Result<GameDefs, DefsError> {
    let data_root = resolve_data_root(root.as_ref())?;
    let mut defs = StagingDefs {
        price_range: load_price_range(&data_root)?,
        ..StagingDefs::default()
    };
    (defs.goods_order, defs.goods) = load_goods(&data_root)?;
    defs.labels = load_labels(&data_root)?;
    attach_icons(&mut defs, load_icons(&data_root)?);
    load_coa_into(&mut defs, &data_root)?;
    defs.production_methods = load_production_methods(&data_root)?;
    defs.buildings = load_buildings(&data_root)?;
    defs.building_groups = load_building_groups(&data_root)?;
    (defs.needs_order, defs.pop_needs) = load_pop_needs(&data_root)?;
    defs.buy_packages = load_buy_packages(&data_root)?;
    defs.obsessions = load_obsessions(&data_root)?;
    defs.resolve()
}

/// Load definitions from an in-memory set of game files.
///
/// Paths may be rooted anywhere, but must contain `common/...` (for example
/// `game/common/goods/00_goods.txt`). Goods localization under
/// `localization/**/goods_l_*.yml` is optional.
pub fn load_from_files(
    files: impl IntoIterator<Item = (String, Vec<u8>)>,
) -> Result<GameDefs, DefsError> {
    let mut builder = DefsBuilder::default();
    builder.add_files(files);
    builder.finish()
}

/// Incremental counterpart to [`load_from_files`].
///
/// A full install offers over 400 MB of coat-of-arms art, which a browser tab
/// cannot buffer in one array and hand to wasm. Feeding it in batches keeps
/// only the current batch alive: textures are decoded and reduced to flag size
/// on arrival, so what the builder retains is a few megabytes of thumbnails.
#[derive(Debug, Default)]
pub struct DefsBuilder {
    /// Clausewitz and localization sources, parsed together once every text
    /// file is in, so a later file still overrides an earlier one by sorted
    /// path no matter what order the batches arrived in.
    texts: Vec<(String, Vec<u8>)>,
    /// Parsed form of `texts`, dropped whenever another text file arrives.
    parsed: Option<ParsedTexts>,
    coa_textures: BTreeMap<String, crate::coa::RgbaImage>,
    icons: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug)]
struct ParsedTexts {
    defs: StagingDefs,
    library: crate::coa::CoaLibrary,
}

impl DefsBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Absorb one batch. Unsupported paths are dropped, as they are on the
    /// one-shot path, so the allowlist stays the trust boundary.
    pub fn add_files(&mut self, files: impl IntoIterator<Item = (String, Vec<u8>)>) {
        for (path, bytes) in files {
            if classify_defs_path(&path, false) != DefsPathClass::Read {
                continue;
            }
            let Some(relative) = normalize_defs_path(&path) else {
                continue;
            };
            if relative.ends_with(".txt") || relative.ends_with(".yml") {
                self.texts.push((relative, bytes));
                self.parsed = None;
            } else if relative.contains("gfx/coat_of_arms/")
                && (relative.ends_with(".dds") || relative.ends_with(".tga"))
            {
                if let (Some(key), Some(image)) = (
                    texture_file_key(&relative),
                    crate::coa::decode_flag_texture(&bytes),
                ) {
                    self.coa_textures.insert(key, image);
                }
            } else if relative.ends_with(".dds") {
                if let (Some(stem), Some(png)) = (icon_stem(&relative), icons::dds_to_png(&bytes)) {
                    self.icons.insert(stem, png);
                }
            }
        }
    }

    /// Parse the text sources, reusing the previous parse when nothing new
    /// arrived since.
    fn parse_texts(&mut self) -> Result<&ParsedTexts, DefsError> {
        if self.parsed.is_none() {
            self.texts.sort_by(|left, right| left.0.cmp(&right.0));
            if !self
                .texts
                .iter()
                .any(|(path, _)| path.starts_with("common/goods/"))
            {
                return Err(DefsError::NotAGameRoot(PathBuf::from("<selected files>")));
            }
            let mut defs = StagingDefs::default();
            let mut library = crate::coa::CoaLibrary::default();
            for (relative, bytes) in &self.texts {
                parse_defs_text(relative, bytes, &mut defs, &mut library)?;
            }
            self.parsed = Some(ParsedTexts { defs, library });
        }
        Ok(self.parsed.as_ref().expect("just parsed"))
    }

    /// Lowercase names of the `gfx` sources the text definitions reference.
    ///
    /// A full install ships far more art than any coat uses — roughly a third
    /// of the emblems and most goods icons go untouched. Letting the caller
    /// ask first means those files are never read at all.
    ///
    /// Call once every text file has been added; art added before this is
    /// still kept, so the answer only ever narrows future reads.
    pub fn needed_gfx_names(&mut self) -> Result<std::collections::BTreeSet<String>, DefsError> {
        let parsed = self.parse_texts()?;
        let mut names = parsed.library.needed_texture_names();
        names.extend(parsed.defs.goods.values().map(|good| {
            good.texture
                .as_deref()
                .and_then(icon_stem)
                .unwrap_or_else(|| good.id.to_lowercase())
        }));
        Ok(names)
    }

    pub fn finish(mut self) -> Result<GameDefs, DefsError> {
        self.parse_texts()?;
        let ParsedTexts { mut defs, library } = self.parsed.take().expect("parsed above");
        let mut library = library;
        attach_icons(&mut defs, self.icons);
        crate::coa::render_library_scaled(&mut library, &self.coa_textures);
        finish_coa(&mut defs, library);
        defs.resolve()
    }
}

/// Fold one Clausewitz or localization file into the definitions under build.
fn parse_defs_text(
    relative: &str,
    bytes: &[u8],
    defs: &mut StagingDefs,
    library: &mut crate::coa::CoaLibrary,
) -> Result<(), DefsError> {
    let path = &PathBuf::from(relative);
    {
        if relative.starts_with("common/defines/") {
            let raw: RawDefinesFile = parse_bytes(path, bytes)?;
            if let Some(value) = raw.price_range() {
                defs.price_range = value;
            }
        } else if relative.starts_with("common/goods/") {
            let (order, goods) = parse_goods_bytes(path, bytes)?;
            for id in order {
                if !defs.goods_order.contains(&id) {
                    defs.goods_order.push(id);
                }
            }
            defs.goods.extend(goods);
        } else if relative.starts_with("common/production_methods/") {
            for method in parse_production_methods_bytes(path, bytes)? {
                defs.production_methods.insert(method.id.clone(), method);
            }
        } else if relative.starts_with("common/buildings/") {
            let raw: BTreeMap<String, RawBuilding> = parse_bytes(path, bytes)?;
            defs.buildings.extend(raw.into_iter().map(|(id, building)| {
                (
                    id.clone(),
                    BuildingType {
                        id,
                        group: building.building_group,
                        city_type: building.city_type,
                    },
                )
            }));
        } else if relative.starts_with("common/building_groups/") {
            let raw: BTreeMap<String, RawBuildingGroup> = parse_bytes(path, bytes)?;
            defs.building_groups
                .extend(raw.into_iter().map(|(id, group)| {
                    (
                        id.clone(),
                        BuildingGroup {
                            id,
                            category: group.category,
                            land_usage: group.land_usage,
                            always_possible: group.always_possible,
                            default_building: group.default_building,
                            parent_group: group.parent_group,
                        },
                    )
                }));
        } else if relative.starts_with("common/pop_needs/") {
            let raw: BTreeMap<String, RawNeed> = parse_bytes(path, bytes)?;
            for (id, need) in raw {
                let entries = need
                    .entry
                    .into_iter()
                    .filter_map(|entry| {
                        entry.goods.map(|good| StagingNeedEntry {
                            good,
                            weight: entry.weight.unwrap_or(1.0),
                            min_supply_share: entry.min_supply_share.unwrap_or(0.0),
                            max_supply_share: entry.max_supply_share.unwrap_or(1.0),
                        })
                    })
                    .collect();
                if !defs.needs_order.contains(&id) {
                    defs.needs_order.push(id.clone());
                }
                defs.pop_needs.insert(
                    id.clone(),
                    StagingNeed {
                        id,
                        default_good: need.default,
                        entries,
                    },
                );
            }
        } else if relative.starts_with("common/buy_packages/") {
            let raw: BTreeMap<String, RawBuyPackage> = parse_bytes(path, bytes)?;
            for (key, package) in raw {
                if let Some(wealth) = parse_wealth_key(&key) {
                    defs.buy_packages.insert(
                        wealth,
                        StagingBuyPackage {
                            wealth,
                            political_strength: package.political_strength.unwrap_or(0.0),
                            needs: package.goods,
                        },
                    );
                }
            }
        } else if relative.starts_with("common/cultures/") {
            let raw: BTreeMap<String, RawCulture> = parse_bytes(path, bytes)?;
            for (culture, value) in raw {
                if !value.obsessions.is_empty() {
                    defs.obsessions.insert(culture, value.obsessions);
                }
            }
        } else if relative.starts_with("common/named_colors/") {
            crate::coa::parse_named_colors(bytes, &mut library.colors);
        } else if relative.starts_with("common/coat_of_arms/template_lists/") {
            crate::coa::parse_template_lists(bytes, &mut library.template_lists);
        } else if relative.starts_with("common/coat_of_arms/") {
            crate::coa::parse_coat_of_arms_file(bytes, &mut library.coats);
        } else if relative.starts_with("common/flag_definitions/") {
            crate::coa::parse_flag_definitions(bytes, &mut library.flag_defs);
        } else if is_english_localization(relative) {
            parse_localization(bytes, &mut defs.labels);
        }
    }
    Ok(())
}

fn load_icons(data_root: &Path) -> Result<BTreeMap<String, Vec<u8>>, DefsError> {
    let mut icons = BTreeMap::new();
    for path in files_with_extension(&data_root.join("gfx/interface/icons/goods_icons"), "dds")? {
        let bytes = std::fs::read(&path).map_err(|source| DefsError::Io {
            path: path.clone(),
            source,
        })?;
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if let (Some(stem), Some(png)) = (icon_stem(&name), icons::dds_to_png(&bytes)) {
            icons.insert(stem, png);
        }
    }
    Ok(icons)
}

/// Key icons by texture file stem, which is how `common/goods` refers to them.
fn icon_stem(path: &str) -> Option<String> {
    let name = path.replace('\\', "/").rsplit('/').next()?.to_lowercase();
    Some(name.strip_suffix(".dds").unwrap_or(&name).to_string())
}

/// Resolve decoded icons against goods, so the blob is keyed by good id.
///
/// Several goods can share one texture, so icons are copied rather than moved.
fn attach_icons(defs: &mut StagingDefs, decoded: BTreeMap<String, Vec<u8>>) {
    if decoded.is_empty() {
        return;
    }
    let wanted = defs
        .goods
        .values()
        .filter_map(|good| {
            let stem = good
                .texture
                .as_deref()
                .and_then(icon_stem)
                .unwrap_or_else(|| good.id.to_lowercase());
            Some((good.id.clone(), decoded.get(&stem)?.clone()))
        })
        .collect::<Vec<_>>();
    defs.icons.extend(wanted);
}

fn load_coa_into(defs: &mut StagingDefs, data_root: &Path) -> Result<(), DefsError> {
    let mut library = crate::coa::CoaLibrary::default();
    for path in txt_files(&data_root.join("common/named_colors"))? {
        let bytes = std::fs::read(&path).map_err(|source| DefsError::Io {
            path: path.clone(),
            source,
        })?;
        crate::coa::parse_named_colors(&bytes, &mut library.colors);
    }
    for path in txt_files(&data_root.join("common/coat_of_arms"))? {
        let bytes = std::fs::read(&path).map_err(|source| DefsError::Io {
            path: path.clone(),
            source,
        })?;
        if path
            .components()
            .any(|component| component.as_os_str() == "template_lists")
        {
            crate::coa::parse_template_lists(&bytes, &mut library.template_lists);
        } else {
            crate::coa::parse_coat_of_arms_file(&bytes, &mut library.coats);
        }
    }
    for path in txt_files(&data_root.join("common/flag_definitions"))? {
        let bytes = std::fs::read(&path).map_err(|source| DefsError::Io {
            path: path.clone(),
            source,
        })?;
        crate::coa::parse_flag_definitions(&bytes, &mut library.flag_defs);
    }
    let mut textures = BTreeMap::new();
    for leaf in ["patterns", "colored_emblems", "textured_emblems"] {
        let dir = data_root.join("gfx/coat_of_arms").join(leaf);
        for ext in ["dds", "tga"] {
            for path in files_with_extension(&dir, ext)? {
                let bytes = std::fs::read(&path).map_err(|source| DefsError::Io {
                    path: path.clone(),
                    source,
                })?;
                if let Some(key) = texture_file_key(&path.to_string_lossy()) {
                    textures.insert(key, bytes);
                }
            }
        }
    }
    crate::coa::render_library(&mut library, &textures);
    finish_coa(defs, library);
    Ok(())
}

fn finish_coa(defs: &mut StagingDefs, library: crate::coa::CoaLibrary) {
    defs.flags = library.rendered;
    defs.flag_defs = library
        .flag_defs
        .into_iter()
        .map(|(tag, flag_defs)| {
            (
                tag,
                flag_defs
                    .into_iter()
                    .map(|def| {
                        let unsupported_trigger =
                            matches!(&def.trigger, crate::coa::FlagTrigger::Unsupported);
                        let any_laws = match def.trigger {
                            crate::coa::FlagTrigger::AnyLaw(laws) => laws,
                            _ => Vec::new(),
                        };
                        crate::FlagDefinition {
                            coa: def.coa,
                            priority: def.priority,
                            any_laws,
                            unsupported_trigger,
                        }
                    })
                    .collect(),
            )
        })
        .collect();
}

fn texture_file_key(path: &str) -> Option<String> {
    let name = path.replace('\\', "/").rsplit('/').next()?.to_lowercase();
    Some(name)
}

fn normalize_defs_path(path: &str) -> Option<String> {
    let path = path.replace('\\', "/");
    for root in ["common/", "localization/", "gfx/"] {
        if let Some(rest) = path.strip_prefix(root) {
            return Some(format!("{root}{rest}"));
        }
        let needle = format!("/{root}");
        if let Some(index) = path.find(&needle) {
            return Some(path[index + 1..].to_string());
        }
    }
    None
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

fn load_goods(data_root: &Path) -> Result<(Vec<String>, BTreeMap<String, Good>), DefsError> {
    let mut order = Vec::new();
    let mut goods = BTreeMap::new();
    for path in txt_files(&data_root.join("common/goods"))? {
        let bytes = std::fs::read(&path).map_err(|source| DefsError::Io {
            path: path.clone(),
            source,
        })?;
        let (file_order, file_goods) = parse_goods_bytes(&path, &bytes)?;
        for id in file_order {
            if !order.contains(&id) {
                order.push(id);
            }
        }
        goods.extend(file_goods);
    }
    Ok((order, goods))
}

fn parse_goods_bytes(
    path: &Path,
    bytes: &[u8],
) -> Result<(Vec<String>, BTreeMap<String, Good>), DefsError> {
    let bytes = strip_bom(bytes);
    if looks_empty(bytes) {
        return Ok((Vec::new(), BTreeMap::new()));
    }
    let tape = TextTape::from_slice(bytes).map_err(|source| DefsError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    let source_order = tape
        .utf8_reader()
        .fields()
        .map(|(key, _, _)| key.read_str().to_string())
        .collect::<Vec<_>>();
    let mut raw: BTreeMap<String, RawGood> = parse_bytes(path, bytes)?;
    let mut order = Vec::new();
    let mut goods = BTreeMap::new();
    for id in source_order {
        let Some(raw_good) = raw.remove(&id) else {
            continue;
        };
        let Some(base_price) = raw_good.base_price() else {
            continue;
        };
        order.push(id.clone());
        goods.insert(
            id.clone(),
            Good {
                id,
                base_price,
                traded_quantity: raw_good
                    .traded_quantity
                    .unwrap_or(crate::DEFAULT_TRADED_QUANTITY),
                texture: raw_good.texture,
            },
        );
    }
    Ok((order, goods))
}

fn load_labels(data_root: &Path) -> Result<BTreeMap<String, String>, DefsError> {
    let mut labels = BTreeMap::new();
    for path in files_with_extension(&data_root.join("localization"), "yml")? {
        let relative = path
            .strip_prefix(data_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if !is_english_localization(&relative) {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|source| DefsError::Io {
            path: path.clone(),
            source,
        })?;
        parse_localization(&bytes, &mut labels);
    }
    Ok(labels)
}

fn is_english_localization(path: &str) -> bool {
    let path = path.replace('\\', "/");
    path.contains("localization/")
        && path.split('/').any(|segment| segment == "english")
        && path.rsplit('/').next().is_some_and(|name| {
            [
                "goods_l_",
                "countries_l_",
                "buildings_l_",
                "building_groups_l_",
                "pop_types_l_",
                "cultures_l_",
                "state_regions_l_",
            ]
            .iter()
            .any(|prefix| name.starts_with(prefix))
                && name.ends_with(".yml")
        })
}

fn parse_localization(bytes: &[u8], labels: &mut BTreeMap<String, String>) {
    let text = String::from_utf8_lossy(strip_bom(bytes));
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.ends_with(':') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        let value = value
            .trim_start_matches(|ch: char| ch.is_ascii_digit())
            .trim();
        let Some(value) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
            continue;
        };
        labels.insert(key.to_string(), value.replace("\\\"", "\""));
    }
}

fn load_production_methods(data_root: &Path) -> Result<BTreeMap<String, StagingPm>, DefsError> {
    let mut pms = BTreeMap::new();
    for path in txt_files(&data_root.join("common/production_methods"))? {
        for pm in parse_production_methods(&path)? {
            pms.insert(pm.id.clone(), pm);
        }
    }
    Ok(pms)
}

fn load_buildings(data_root: &Path) -> Result<BTreeMap<String, BuildingType>, DefsError> {
    let mut buildings = BTreeMap::new();
    for path in txt_files(&data_root.join("common/buildings"))? {
        let file: BTreeMap<String, RawBuilding> = parse_file(&path)?;
        buildings.extend(file.into_iter().map(|(id, raw)| {
            (
                id.clone(),
                BuildingType {
                    id,
                    group: raw.building_group,
                    city_type: raw.city_type,
                },
            )
        }));
    }
    Ok(buildings)
}

fn load_building_groups(data_root: &Path) -> Result<BTreeMap<String, BuildingGroup>, DefsError> {
    let mut groups = BTreeMap::new();
    for path in txt_files(&data_root.join("common/building_groups"))? {
        let file: BTreeMap<String, RawBuildingGroup> = parse_file(&path)?;
        groups.extend(file.into_iter().map(|(id, raw)| {
            (
                id.clone(),
                BuildingGroup {
                    id,
                    category: raw.category,
                    land_usage: raw.land_usage,
                    always_possible: raw.always_possible,
                    default_building: raw.default_building,
                    parent_group: raw.parent_group,
                },
            )
        }));
    }
    Ok(groups)
}

fn parse_production_methods(path: &Path) -> Result<Vec<StagingPm>, DefsError> {
    let bytes = std::fs::read(path).map_err(|source| DefsError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_production_methods_bytes(path, &bytes)
}

fn parse_production_methods_bytes(path: &Path, bytes: &[u8]) -> Result<Vec<StagingPm>, DefsError> {
    let bytes = strip_bom(bytes);
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
        out.push(StagingPm {
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

fn load_pop_needs(
    data_root: &Path,
) -> Result<(Vec<String>, BTreeMap<String, StagingNeed>), DefsError> {
    let mut order = Vec::new();
    let mut needs = BTreeMap::new();
    for path in txt_files(&data_root.join("common/pop_needs"))? {
        let file: BTreeMap<String, RawNeed> = parse_file(&path)?;
        for (id, raw) in file {
            let entries = raw
                .entry
                .into_iter()
                .filter_map(|e| {
                    e.goods.map(|good| StagingNeedEntry {
                        good,
                        weight: e.weight.unwrap_or(1.0),
                        min_supply_share: e.min_supply_share.unwrap_or(0.0),
                        max_supply_share: e.max_supply_share.unwrap_or(1.0),
                    })
                })
                .collect();
            if !order.contains(&id) {
                order.push(id.clone());
            }
            needs.insert(
                id.clone(),
                StagingNeed {
                    id,
                    default_good: raw.default,
                    entries,
                },
            );
        }
    }
    Ok((order, needs))
}

fn load_buy_packages(data_root: &Path) -> Result<BTreeMap<u8, StagingBuyPackage>, DefsError> {
    let mut packages = BTreeMap::new();
    for path in txt_files(&data_root.join("common/buy_packages"))? {
        let file: BTreeMap<String, RawBuyPackage> = parse_file(&path)?;
        for (key, raw) in file {
            let Some(wealth) = parse_wealth_key(&key) else {
                continue;
            };
            packages.insert(
                wealth,
                StagingBuyPackage {
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
    files_with_extension(dir, "txt")
}

fn files_with_extension(dir: &Path, extension: &str) -> Result<Vec<PathBuf>, DefsError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    collect_files(dir, extension, &mut files).map_err(|source| DefsError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    files.sort();
    Ok(files)
}

fn collect_files(dir: &Path, extension: &str, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, extension, files)?;
        } else if path.extension().is_some_and(|ext| ext == extension) {
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
    parse_bytes(path, &bytes)
}

fn parse_bytes<T>(path: &Path, bytes: &[u8]) -> Result<T, DefsError>
where
    T: DeserializeOwned + Default,
{
    let bytes = strip_bom(bytes);
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
    traded_quantity: Option<f64>,
    texture: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawBuilding {
    building_group: Option<String>,
    city_type: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawBuildingGroup {
    category: Option<String>,
    land_usage: Option<String>,
    #[serde(default)]
    always_possible: bool,
    default_building: Option<String>,
    parent_group: Option<String>,
}

impl RawGood {
    fn base_price(&self) -> Option<f64> {
        self.cost.or(self.base_price)
    }
}

fn goods_modifier_key(key: &str) -> Option<(&str, bool)> {
    let (rest, is_output) = if let Some(rest) = key.strip_prefix("goods_output_") {
        (rest, true)
    } else {
        let rest = key.strip_prefix("goods_input_")?;
        (rest, false)
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
