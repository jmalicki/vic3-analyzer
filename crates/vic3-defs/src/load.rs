//! Load Clausewitz definition trees into [`crate::GameDefs`].
//!
//! # Install vs fixture
//!
//! [`load_from_path`] accepts either:
//! - a **game install** whose data lives under `game/` (Steam / PDX launcher)
//! - a **fixture** / unpacked `game` directory that already has `common/` at
//!   its root
//!
//! Resolution looks for `common/goods` (or `game/common/goods`) and fails with
//! [`crate::DefsError::NotAGameRoot`] otherwise.
//!
//! # In-memory / wasm path
//!
//! [`load_from_files`] and [`DefsBuilder`] apply [`crate::classify_defs_path`] as
//! the trust boundary, then resolve string good/need ids into dense indices via
//! staging. Prefer [`DefsBuilder`] when CoA art must arrive in batches so a
//! browser tab never buffers hundreds of megabytes at once.
//!
//! # Expected layout
//!
//! Relative paths under the data root (also listed on the crate root):
//! - `common/goods`, `common/defines`, `common/production_methods`, …
//! - `gfx/coat_of_arms/{patterns,colored_emblems,textured_emblems}`
//! - allowlisted `gfx/interface/icons/…` leaf dirs
//! - `localization/english/*_l_*_english.yml` for selected prefixes
//!
//! All `*.txt` files in those directories merge in sorted path order; later
//! files override the same id.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use jomini::text::{ObjectReader, Operator};
use jomini::{Encoding, JominiDeserialize, TextTape};
use serde::de::{self, DeserializeOwned, Visitor};
use serde::{Deserialize, Deserializer};

use crate::{
    classify_defs_path, icons,
    path_rules::{extra_icon_kind, ICON_LEAFS, LOCALIZATION_PREFIXES},
    staging::{StagingBuyPackage, StagingDefs, StagingNeed, StagingNeedEntry, StagingPm},
    BuildingGroup, BuildingType, DefsError, DefsPathClass, GameDefs, Good, PopType,
    QualificationFactors, Technology, DEFAULT_PRICE_RANGE,
};

/// Load definitions from a Victoria 3 install or a fixture tree.
///
/// See the module overview for install vs fixture roots. Relative paths under
/// the data root:
/// - `common/goods` — good id → `cost` (base price); `base_price` is also accepted
/// - `common/defines` — `NEconomy.PRICE_RANGE` (also `NDefines.NEconomy` or top-level)
/// - `common/production_methods` — PM ids plus `goods_input_*` / `goods_output_*`
/// - `common/production_method_groups` — group id → production method ids
/// - `common/buildings` — building type → group / city type / PM groups
/// - `common/script_values` — flat numeric constants (e.g. construction costs)
/// - `common/building_groups` — category, land usage, and default building
/// - `common/technology/technologies` — tech cost / `unlocking_technologies`
/// - `common/pop_types` — profession qualification scripts (static analysis)
/// - `common/pop_needs` — need substitution tables (`entry` / min & max supply share)
/// - `common/buy_packages` — `wealth_N` packages
/// - `common/cultures` — optional `obsessions = { good_id ... }` (empty is fine)
///
/// All `*.txt` files in those directories are merged in sorted path order; later
/// files override the same id.
///
/// # Errors
///
/// - [`DefsError::NotAGameRoot`] — neither `common/goods` nor `game/common/goods`
/// - [`DefsError::Io`] / [`DefsError::Parse`] — read or Clausewitz failures
pub fn load_from_path(root: impl AsRef<Path>) -> Result<GameDefs, DefsError> {
    let data_root = resolve_data_root(root.as_ref())?;
    let mut defs = StagingDefs {
        price_range: load_price_range(&data_root)?,
        ..StagingDefs::default()
    };
    (defs.goods_order, defs.goods) = load_goods(&data_root)?;
    defs.labels = load_labels(&data_root)?;
    defs.production_methods = load_production_methods(&data_root)?;
    defs.production_method_groups = load_production_method_groups(&data_root)?;
    defs.script_values = load_script_values(&data_root)?;
    (defs.building_types_order, defs.building_types) =
        load_building_types(&data_root, &mut defs.building_construction_refs)?;
    defs.building_groups = load_building_groups(&data_root)?;
    defs.technologies = load_technologies(&data_root)?;
    defs.pop_types = load_pop_types(&data_root)?;
    let (goods_icons, extra_icons) = load_icons(&data_root)?;
    attach_icons(&mut defs, goods_icons);
    attach_extra_icons(&mut defs, extra_icons);
    load_coa_into(&mut defs, &data_root)?;
    (defs.needs_order, defs.pop_needs) = load_pop_needs(&data_root)?;
    defs.buy_packages = load_buy_packages(&data_root)?;
    defs.obsessions = load_obsessions(&data_root)?;
    defs.resolve()
}

/// Load definitions from an in-memory set of game files.
///
/// Paths may be rooted anywhere, but must contain `common/...` (for example
/// `game/common/goods/00_goods.txt`). Goods localization under
/// `localization/**/goods_l_*.yml` is optional. Unsupported paths are dropped
/// via [`crate::classify_defs_path`].
///
/// # Errors
///
/// - [`DefsError::NotAGameRoot`] — no `common/goods` file in the selection
/// - [`DefsError::Parse`] — Clausewitz / localization parse failure
pub fn load_from_files(
    files: impl IntoIterator<Item = (String, Vec<u8>)>,
) -> Result<GameDefs, DefsError> {
    let mut builder = DefsBuilder::default();
    builder.add_files(files)?;
    builder.finish()
}

/// Incremental counterpart to [`load_from_files`].
///
/// A full install offers over 400 MB of coat-of-arms art, which a browser tab
/// cannot buffer in one array and hand to wasm. Feeding it in batches keeps
/// only the current batch alive: textures are decoded and reduced to flag size
/// on arrival, so what the builder retains is a few megabytes of thumbnails.
/// Clausewitz text is parsed as each file arrives; coats render as soon as
/// their textures are in, so `finish` is encoding rather than a second parse.
#[derive(Debug, Default)]
pub struct DefsBuilder {
    /// Per-file parse results, keyed by normalized path so a later file still
    /// overrides an earlier one by sorted path no matter what order the
    /// batches arrived in.
    texts: BTreeMap<String, ParsedTextFile>,
    /// Merged form of `texts`, dropped whenever another text file arrives.
    parsed: Option<ParsedTexts>,
    /// Parents and template lists have been folded; coats may render as art
    /// arrives.
    coats_prepared: bool,
    coa_textures: BTreeMap<String, crate::coa::RgbaImage>,
    icons: BTreeMap<String, Vec<u8>>,
    extra_icons: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Clone)]
struct ParsedTextFile {
    price_range: Option<f64>,
    defs: StagingDefs,
    library: crate::coa::CoaLibrary,
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
    /// one-shot path, so the allowlist stays the trust boundary. Text is
    /// parsed here; a later merge in path order still applies overrides.
    ///
    /// # Errors
    ///
    /// Returns [`DefsError::Parse`] when an allowlisted text file is invalid.
    pub fn add_files(
        &mut self,
        files: impl IntoIterator<Item = (String, Vec<u8>)>,
    ) -> Result<(), DefsError> {
        for (path, bytes) in files {
            if classify_defs_path(&path, false) != DefsPathClass::Read {
                continue;
            }
            let Some(relative) = normalize_defs_path(&path) else {
                continue;
            };
            if relative.ends_with(".txt") || relative.ends_with(".yml") {
                self.texts
                    .insert(relative.clone(), parse_text_file(&relative, &bytes)?);
                self.parsed = None;
                self.coats_prepared = false;
            } else if relative.contains("gfx/coat_of_arms/")
                && (relative.ends_with(".dds") || relative.ends_with(".tga"))
            {
                if let (Some(key), Some(image)) = (
                    texture_file_key(&relative),
                    crate::coa::decode_flag_texture(&bytes),
                ) {
                    self.coa_textures.insert(key, image);
                    self.render_ready_coats();
                }
            } else if relative.ends_with(".dds") {
                if let (Some(stem), Some(png)) = (icon_stem(&relative), icons::dds_to_png(&bytes)) {
                    if let Some(kind) = extra_icon_kind(&relative) {
                        self.extra_icons.insert(format!("{kind}:{stem}"), png);
                    } else {
                        self.icons.insert(stem, png);
                    }
                }
            }
        }
        Ok(())
    }

    /// Merge per-file parses in path order, reusing the previous merge when
    /// nothing new arrived since.
    fn merge_texts(&mut self) -> Result<(), DefsError> {
        if self.parsed.is_some() {
            return Ok(());
        }
        if !self
            .texts
            .keys()
            .any(|path| path.starts_with("common/goods/"))
        {
            return Err(DefsError::NotAGameRoot(PathBuf::from("<selected files>")));
        }
        let mut defs = StagingDefs::default();
        let mut library = crate::coa::CoaLibrary::default();
        for file in self.texts.values() {
            absorb_text_file(&mut defs, &mut library, file);
        }
        self.parsed = Some(ParsedTexts { defs, library });
        self.coats_prepared = false;
        Ok(())
    }

    fn prepare_coats(&mut self) -> Result<(), DefsError> {
        self.merge_texts()?;
        if self.coats_prepared {
            return Ok(());
        }
        let mut parsed = self.parsed.take().expect("merged above");
        parsed.library.prepare_render();
        parsed.library.render_ready_coats(&self.coa_textures);
        self.parsed = Some(parsed);
        self.coats_prepared = true;
        Ok(())
    }

    fn render_ready_coats(&mut self) {
        if !self.coats_prepared {
            return;
        }
        let Some(mut parsed) = self.parsed.take() else {
            return;
        };
        parsed.library.render_ready_coats(&self.coa_textures);
        self.parsed = Some(parsed);
    }

    /// Lowercase names of the `gfx` sources the text definitions reference.
    ///
    /// A full install ships far more art than any coat uses — roughly a third
    /// of the emblems and most goods icons go untouched. Letting the caller
    /// ask first means those files are never read at all.
    ///
    /// Call once every text file has been added; art added before this is
    /// still kept, so the answer only ever narrows future reads. Coats that
    /// already have their textures start rendering here.
    pub fn needed_gfx_names(&mut self) -> Result<std::collections::BTreeSet<String>, DefsError> {
        self.merge_texts()?;
        let parsed = self.parsed.as_ref().expect("merged above");
        let mut names = parsed.library.needed_texture_names();
        names.extend(parsed.defs.goods.values().map(|good| {
            good.texture
                .as_deref()
                .and_then(icon_stem)
                .unwrap_or_else(|| good.id.to_lowercase())
        }));
        names.extend(
            parsed
                .defs
                .building_types
                .keys()
                .map(|id| id.to_lowercase()),
        );
        names.extend(parsed.defs.production_methods.values().filter_map(|pm| {
            pm.texture
                .as_deref()
                .and_then(icon_stem)
                .or_else(|| Some(pm.id.to_lowercase()))
        }));
        self.prepare_coats()?;
        Ok(names)
    }

    /// Merge parses, render remaining coats, and resolve string ids into
    /// dense [`crate::GoodId`] / [`crate::NeedId`] indices.
    ///
    /// # Errors
    ///
    /// Same class of failures as [`load_from_files`].
    pub fn finish(mut self) -> Result<GameDefs, DefsError> {
        self.merge_texts()?;
        let ParsedTexts {
            mut defs,
            mut library,
        } = self.parsed.take().expect("merged above");
        if !self.coats_prepared {
            library.prepare_render();
        }
        library.render_remaining_coats(&self.coa_textures);
        attach_icons(&mut defs, self.icons);
        attach_extra_icons(&mut defs, self.extra_icons);
        finish_coa(&mut defs, library);
        defs.resolve()
    }
}

fn parse_text_file(relative: &str, bytes: &[u8]) -> Result<ParsedTextFile, DefsError> {
    let mut defs = StagingDefs::default();
    let mut library = crate::coa::CoaLibrary::default();
    let price_range = parse_defs_text(relative, bytes, &mut defs, &mut library)?;
    Ok(ParsedTextFile {
        price_range,
        defs,
        library,
    })
}

fn absorb_text_file(
    defs: &mut StagingDefs,
    library: &mut crate::coa::CoaLibrary,
    file: &ParsedTextFile,
) {
    if let Some(value) = file.price_range {
        defs.price_range = value;
    }
    for id in &file.defs.goods_order {
        if !defs.goods_order.contains(id) {
            defs.goods_order.push(id.clone());
        }
    }
    for id in &file.defs.building_types_order {
        if !defs.building_types_order.contains(id) {
            defs.building_types_order.push(id.clone());
        }
    }
    for id in &file.defs.needs_order {
        if !defs.needs_order.contains(id) {
            defs.needs_order.push(id.clone());
        }
    }
    defs.goods.extend(file.defs.goods.clone());
    defs.labels.extend(file.defs.labels.clone());
    defs.production_methods
        .extend(file.defs.production_methods.clone());
    defs.production_method_groups
        .extend(file.defs.production_method_groups.clone());
    for (id, building) in &file.defs.building_types {
        if building.required_construction.is_some() {
            defs.building_construction_refs.remove(id);
        }
    }
    defs.building_types.extend(file.defs.building_types.clone());
    defs.building_construction_refs
        .extend(file.defs.building_construction_refs.clone());
    defs.script_values.extend(file.defs.script_values.clone());
    defs.building_groups
        .extend(file.defs.building_groups.clone());
    defs.technologies.extend(file.defs.technologies.clone());
    defs.pop_types.extend(file.defs.pop_types.clone());
    defs.pop_needs.extend(file.defs.pop_needs.clone());
    defs.buy_packages.extend(file.defs.buy_packages.clone());
    defs.obsessions.extend(file.defs.obsessions.clone());
    library.colors.extend(file.library.colors.clone());
    library.coats.extend(file.library.coats.clone());
    library
        .template_lists
        .extend(file.library.template_lists.clone());
    library.flag_defs.extend(file.library.flag_defs.clone());
}

/// Fold one Clausewitz or localization file into the definitions under build.
fn parse_defs_text(
    relative: &str,
    bytes: &[u8],
    defs: &mut StagingDefs,
    library: &mut crate::coa::CoaLibrary,
) -> Result<Option<f64>, DefsError> {
    let path = &PathBuf::from(relative);
    let mut price_range = None;
    {
        if relative.starts_with("common/defines/") {
            let raw: RawDefinesFile = parse_bytes(path, bytes)?;
            if let Some(value) = raw.price_range() {
                defs.price_range = value;
                price_range = Some(value);
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
        } else if relative.starts_with("common/production_method_groups/") {
            defs.production_method_groups
                .extend(parse_production_method_groups_bytes(path, bytes)?);
        } else if relative.starts_with("common/script_values/") {
            defs.script_values
                .extend(parse_script_values_bytes(path, bytes)?);
        } else if relative.starts_with("common/buildings/") {
            let (file_order, file_buildings, construction_updates) =
                parse_building_types_bytes(path, bytes)?;
            for id in file_order {
                if !defs.building_types_order.contains(&id) {
                    defs.building_types_order.push(id);
                }
            }
            for (id, construction_ref) in construction_updates {
                match construction_ref {
                    Some(name) => {
                        defs.building_construction_refs.insert(id, name);
                    }
                    None => {
                        defs.building_construction_refs.remove(&id);
                    }
                }
            }
            defs.building_types.extend(file_buildings);
        } else if relative.starts_with("common/technology/technologies/") {
            defs.technologies
                .extend(parse_technologies_bytes(path, bytes)?);
        } else if relative.starts_with("common/pop_types/") {
            defs.pop_types.extend(parse_pop_types_bytes(path, bytes)?);
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
    Ok(price_range)
}

type IconPngs = BTreeMap<String, Vec<u8>>;

fn load_icons(data_root: &Path) -> Result<(IconPngs, IconPngs), DefsError> {
    let mut goods = BTreeMap::new();
    let mut extra = BTreeMap::new();
    let icons_root = data_root.join("gfx/interface/icons");
    for leaf in ICON_LEAFS {
        for path in files_with_extension(&icons_root.join(leaf), "dds")? {
            let bytes = std::fs::read(&path).map_err(|source| DefsError::Io {
                path: path.clone(),
                source,
            })?;
            let relative = path.to_string_lossy();
            let Some(stem) = icon_stem(&relative) else {
                continue;
            };
            let Some(png) = icons::dds_to_png(&bytes) else {
                continue;
            };
            if let Some(kind) = extra_icon_kind(&relative) {
                extra.insert(format!("{kind}:{stem}"), png);
            } else {
                goods.insert(stem, png);
            }
        }
    }
    Ok((goods, extra))
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

/// Store namespaced extra icons and alias script ids onto texture stems.
fn attach_extra_icons(defs: &mut StagingDefs, decoded: BTreeMap<String, Vec<u8>>) {
    if decoded.is_empty() {
        return;
    }
    defs.extra_icons.extend(decoded);
    let mut aliases = Vec::new();
    for id in defs.building_types.keys() {
        let key = format!("building:{id}");
        if defs.extra_icons.contains_key(&key) {
            continue;
        }
        let stem = id.strip_prefix("building_").unwrap_or(id);
        if let Some(png) = defs.extra_icons.get(&format!("building:{stem}")).cloned() {
            aliases.push((key, png));
        }
    }
    for pm in defs.production_methods.values() {
        let key = format!("pm:{}", pm.id);
        if defs.extra_icons.contains_key(&key) {
            continue;
        }
        let stem = pm
            .texture
            .as_deref()
            .and_then(icon_stem)
            .unwrap_or_else(|| pm.id.strip_prefix("pm_").unwrap_or(&pm.id).to_lowercase());
        if let Some(png) = defs.extra_icons.get(&format!("pm:{stem}")).cloned() {
            aliases.push((key, png));
        }
    }
    defs.extra_icons.extend(aliases);
    copy_extra(&mut defs.extra_icons, "military:army", "military:army_01");
    copy_extra(&mut defs.extra_icons, "military:navy", "military:fleet_01");
    copy_extra(&mut defs.extra_icons, "military:fleet", "military:fleet_01");
    copy_extra(
        &mut defs.extra_icons,
        "military:battalions",
        "generic:battalions",
    );
    copy_extra(&mut defs.extra_icons, "alert:starvation", "alert:starving");
    copy_extra(
        &mut defs.extra_icons,
        "alert:market",
        "alert:goods_shortage",
    );
    copy_extra(
        &mut defs.extra_icons,
        "alert:market_access",
        "generic:world_market_access",
    );
    copy_extra(
        &mut defs.extra_icons,
        "alert:world_market_access",
        "generic:world_market_access",
    );
    copy_extra(
        &mut defs.extra_icons,
        "alert:qualification",
        "generic:literacy",
    );
    copy_extra(&mut defs.extra_icons, "alert:literacy", "generic:literacy");
    copy_extra(
        &mut defs.extra_icons,
        "alert:population",
        "generic:population",
    );
    copy_extra(
        &mut defs.extra_icons,
        "alert:unemployment",
        "generic:population",
    );
}

fn copy_extra(extra: &mut BTreeMap<String, Vec<u8>>, dest: &str, src: &str) {
    if extra.contains_key(dest) {
        return;
    }
    if let Some(png) = extra.get(src).cloned() {
        extra.insert(dest.to_string(), png);
    }
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
            LOCALIZATION_PREFIXES
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

fn load_script_values(data_root: &Path) -> Result<BTreeMap<String, f64>, DefsError> {
    let mut values = BTreeMap::new();
    for path in txt_files(&data_root.join("common/script_values"))? {
        let bytes = std::fs::read(&path).map_err(|source| DefsError::Io {
            path: path.clone(),
            source,
        })?;
        values.extend(parse_script_values_bytes(&path, &bytes)?);
    }
    Ok(values)
}

/// Collect top-level numeric script values; nested formula blocks are skipped.
fn parse_script_values_bytes(
    path: &Path,
    bytes: &[u8],
) -> Result<BTreeMap<String, f64>, DefsError> {
    let bytes = strip_bom(bytes);
    if looks_empty(bytes) {
        return Ok(BTreeMap::new());
    }
    let tape = TextTape::from_slice(bytes).map_err(|source| DefsError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    let reader = tape.utf8_reader();
    let mut out = BTreeMap::new();
    for (key, _op, value) in reader.fields() {
        let id = key.read_str().to_string();
        if let Some(num) = value.read_scalar().ok().and_then(|s| s.to_f64().ok()) {
            if num.is_finite() {
                out.insert(id, num);
            }
        }
    }
    Ok(out)
}

fn load_building_types(
    data_root: &Path,
    construction_refs: &mut BTreeMap<String, String>,
) -> Result<(Vec<String>, BTreeMap<String, BuildingType>), DefsError> {
    let mut order = Vec::new();
    let mut buildings = BTreeMap::new();
    for path in txt_files(&data_root.join("common/buildings"))? {
        let bytes = std::fs::read(&path).map_err(|source| DefsError::Io {
            path: path.clone(),
            source,
        })?;
        let (file_order, file_buildings, construction_updates) =
            parse_building_types_bytes(&path, &bytes)?;
        for id in file_order {
            if !order.contains(&id) {
                order.push(id);
            }
        }
        for (id, construction_ref) in construction_updates {
            match construction_ref {
                Some(name) => {
                    construction_refs.insert(id, name);
                }
                None => {
                    construction_refs.remove(&id);
                }
            }
        }
        buildings.extend(file_buildings);
    }
    Ok((order, buildings))
}

type ParsedBuildingTypes = (
    Vec<String>,
    BTreeMap<String, BuildingType>,
    BTreeMap<String, Option<String>>,
);

fn parse_building_types_bytes(path: &Path, bytes: &[u8]) -> Result<ParsedBuildingTypes, DefsError> {
    let bytes = strip_bom(bytes);
    if looks_empty(bytes) {
        return Ok((Vec::new(), BTreeMap::new(), BTreeMap::new()));
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
    let mut raw: BTreeMap<String, RawBuilding> = parse_bytes(path, bytes)?;
    let mut order = Vec::new();
    let mut buildings = BTreeMap::new();
    let mut construction_updates = BTreeMap::new();
    for id in source_order {
        let Some(raw_building) = raw.remove(&id) else {
            continue;
        };
        let (required_construction, construction_ref) =
            resolve_raw_construction(&raw_building.required_construction);
        construction_updates.insert(id.clone(), construction_ref);
        order.push(id.clone());
        buildings.insert(
            id.clone(),
            BuildingType {
                id,
                group: raw_building.building_group,
                city_type: raw_building.city_type,
                production_method_groups: raw_building.production_method_groups,
                required_construction,
            },
        );
    }
    Ok((order, buildings, construction_updates))
}

fn resolve_raw_construction(raw: &Option<NumberOrIdent>) -> (Option<f64>, Option<String>) {
    match raw {
        Some(NumberOrIdent::Number(v)) if v.is_finite() && *v >= 0.0 => (Some(*v), None),
        Some(NumberOrIdent::Ident(name)) => {
            // jomini text often yields unquoted numbers as idents.
            if let Ok(v) = name.parse::<f64>() {
                if v.is_finite() && v >= 0.0 {
                    return (Some(v), None);
                }
            }
            (None, Some(name.clone()))
        }
        _ => (None, None),
    }
}

fn load_technologies(data_root: &Path) -> Result<BTreeMap<String, Technology>, DefsError> {
    let mut technologies = BTreeMap::new();
    for path in txt_files(&data_root.join("common/technology/technologies"))? {
        technologies.extend(parse_technologies_file(&path)?);
    }
    Ok(technologies)
}

fn parse_technologies_file(path: &Path) -> Result<BTreeMap<String, Technology>, DefsError> {
    let bytes = std::fs::read(path).map_err(|source| DefsError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_technologies_bytes(path, &bytes)
}

fn parse_technologies_bytes(
    path: &Path,
    bytes: &[u8],
) -> Result<BTreeMap<String, Technology>, DefsError> {
    let bytes = strip_bom(bytes);
    if looks_empty(bytes) {
        return Ok(BTreeMap::new());
    }
    let raw: BTreeMap<String, RawTechnology> = parse_bytes(path, bytes)?;
    Ok(raw
        .into_iter()
        .map(|(id, tech)| {
            (
                id.clone(),
                Technology {
                    id,
                    cost: tech.cost.filter(|v| v.is_finite() && *v >= 0.0),
                    prerequisites: tech.unlocking_technologies,
                },
            )
        })
        .collect())
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
        let mut employment = BTreeMap::new();
        let mut education_access = false;
        let mut qualifications_boost = false;
        let mut country_construction_add = None;
        let mut texture = None;
        if let Ok(obj) = value.read_object() {
            for (field, _op, field_value) in obj.fields() {
                if field.read_str().as_ref() == "texture" {
                    texture = field_value
                        .read_scalar()
                        .ok()
                        .map(|scalar| scalar.to_string().trim_matches('"').to_string());
                }
            }
            collect_pm_modifiers(
                &obj,
                &mut inputs,
                &mut outputs,
                &mut employment,
                &mut education_access,
                &mut qualifications_boost,
                &mut country_construction_add,
            );
        }
        out.push(StagingPm {
            id,
            texture,
            inputs,
            outputs,
            employment,
            education_access,
            qualifications_boost,
            country_construction_add,
        });
    }
    Ok(out)
}

fn collect_pm_modifiers<E: Encoding + Clone>(
    obj: &ObjectReader<'_, '_, E>,
    inputs: &mut BTreeMap<String, f64>,
    outputs: &mut BTreeMap<String, f64>,
    employment: &mut BTreeMap<String, f64>,
    education_access: &mut bool,
    qualifications_boost: &mut bool,
    country_construction_add: &mut Option<f64>,
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
        if let Some(profession) = employment_modifier_key(name.as_ref()) {
            if let Some(num) = value.read_scalar().ok().and_then(|s| s.to_f64().ok()) {
                *employment.entry(profession.to_string()).or_insert(0.0) += num;
            }
            continue;
        }
        if name.as_ref() == "country_construction_add" {
            if let Some(num) = value.read_scalar().ok().and_then(|s| s.to_f64().ok()) {
                if num.is_finite() {
                    *country_construction_add = Some(country_construction_add.unwrap_or(0.0) + num);
                }
            }
            continue;
        }
        let lower = name.to_ascii_lowercase();
        if lower.contains("education_access") {
            *education_access = true;
        }
        if lower.contains("qualifications") {
            *qualifications_boost = true;
        }
        if let Ok(nested) = value.read_object() {
            collect_pm_modifiers(
                &nested,
                inputs,
                outputs,
                employment,
                education_access,
                qualifications_boost,
                country_construction_add,
            );
        }
    }
}

fn employment_modifier_key(key: &str) -> Option<&str> {
    let rest = key.strip_prefix("building_employment_")?;
    Some(rest.strip_suffix("_add").unwrap_or(rest)).filter(|id| !id.is_empty())
}

fn load_production_method_groups(
    data_root: &Path,
) -> Result<BTreeMap<String, Vec<String>>, DefsError> {
    let mut groups = BTreeMap::new();
    for path in txt_files(&data_root.join("common/production_method_groups"))? {
        let bytes = std::fs::read(&path).map_err(|source| DefsError::Io {
            path: path.clone(),
            source,
        })?;
        groups.extend(parse_production_method_groups_bytes(&path, &bytes)?);
    }
    Ok(groups)
}

fn parse_production_method_groups_bytes(
    path: &Path,
    bytes: &[u8],
) -> Result<BTreeMap<String, Vec<String>>, DefsError> {
    let bytes = strip_bom(bytes);
    if looks_empty(bytes) {
        return Ok(BTreeMap::new());
    }
    let file: BTreeMap<String, RawPmg> = parse_bytes(path, bytes)?;
    Ok(file
        .into_iter()
        .map(|(id, raw)| (id, raw.production_methods))
        .collect())
}

fn load_pop_types(data_root: &Path) -> Result<BTreeMap<String, PopType>, DefsError> {
    let mut pop_types = BTreeMap::new();
    for path in txt_files(&data_root.join("common/pop_types"))? {
        let bytes = std::fs::read(&path).map_err(|source| DefsError::Io {
            path: path.clone(),
            source,
        })?;
        pop_types.extend(parse_pop_types_bytes(&path, &bytes)?);
    }
    Ok(pop_types)
}

/// Walk `qualifications = { ... }` for `is_pop_type`, `literacy`, and `wealth`.
/// This is not a Vic3 scripted-value interpreter.
fn parse_pop_types_bytes(
    path: &Path,
    bytes: &[u8],
) -> Result<BTreeMap<String, PopType>, DefsError> {
    let bytes = strip_bom(bytes);
    if looks_empty(bytes) {
        return Ok(BTreeMap::new());
    }
    let tape = TextTape::from_slice(bytes).map_err(|source| DefsError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    let reader = tape.utf8_reader();
    let mut out = BTreeMap::new();
    for (key, _op, value) in reader.fields() {
        let id = key.read_str().to_string();
        let mut can_always_hire = false;
        let mut qualifications = QualificationFactors::default();
        if let Ok(obj) = value.read_object() {
            for (field, _op, field_value) in obj.fields() {
                let name = field.read_str();
                if name.as_ref() == "can_always_hire" {
                    can_always_hire = field_value
                        .read_scalar()
                        .ok()
                        .is_some_and(|scalar| scalar.to_string() == "yes");
                } else if name.as_ref() == "qualifications" {
                    if let Ok(block) = field_value.read_object() {
                        walk_qualifications(&block, &mut qualifications, None, false);
                    }
                }
            }
        }
        out.insert(
            id.clone(),
            PopType {
                id,
                can_always_hire,
                qualifications,
            },
        );
    }
    Ok(out)
}

fn walk_qualifications<E: Encoding + Clone>(
    obj: &ObjectReader<'_, '_, E>,
    factors: &mut QualificationFactors,
    current_source: Option<String>,
    in_wealth: bool,
) -> Option<String> {
    let mut source = current_source;
    let mut wealth_here = in_wealth;
    for (key, op, value) in obj.fields() {
        let name = key.read_str();
        let name = name.as_ref();
        if name == "is_pop_type" || name == "pop_type" {
            if let Some(prof) = value
                .read_scalar()
                .ok()
                .map(|scalar| scalar.to_string().trim_matches('"').to_string())
                .filter(|id| !id.is_empty())
            {
                factors
                    .source_multipliers
                    .entry(prof.clone())
                    .or_insert(1.0);
                source = Some(prof);
            }
            continue;
        }
        if name.eq_ignore_ascii_case("literacy") {
            factors.literacy = true;
        }
        if name.eq_ignore_ascii_case("wealth") {
            factors.wealth = true;
            wealth_here = true;
        }
        if let Ok(scalar) = value.read_scalar() {
            let text = scalar.to_string();
            if text.eq_ignore_ascii_case("literacy") {
                factors.literacy = true;
            }
            if text.eq_ignore_ascii_case("wealth") {
                factors.wealth = true;
                wealth_here = true;
            }
            if let Ok(num) = scalar.to_f64() {
                if name == "multiply" {
                    if let Some(prof) = source.as_deref() {
                        factors.source_multipliers.insert(prof.to_string(), num);
                    }
                }
                if name == "subtract" && wealth_here && factors.wealth_floor.is_none() {
                    factors.wealth_floor = Some(num);
                }
                // `wealth < N` / `wealth <= N` gates in limit blocks.
                if name.eq_ignore_ascii_case("wealth")
                    && matches!(op, Some(Operator::LessThan) | Some(Operator::LessThanEqual))
                    && factors.wealth_floor.is_none()
                {
                    factors.wealth_floor = Some(num);
                }
            }
        }
        if let Ok(nested) = value.read_object() {
            if name == "multiply" {
                if let Some(num) = object_numeric_value(&nested) {
                    if let Some(prof) = source.as_deref() {
                        factors.source_multipliers.insert(prof.to_string(), num);
                    }
                }
            }
            if name == "subtract" && wealth_here && factors.wealth_floor.is_none() {
                if let Some(num) = object_numeric_value(&nested) {
                    factors.wealth_floor = Some(num);
                }
            }
            if let Some(child) = walk_qualifications(&nested, factors, source.clone(), wealth_here)
            {
                source = Some(child);
                if name == "multiply" {
                    // already applied above if numeric
                }
            }
        }
    }
    source
}

fn object_numeric_value<E: Encoding + Clone>(obj: &ObjectReader<'_, '_, E>) -> Option<f64> {
    for (key, _op, value) in obj.fields() {
        if key.read_str().as_ref() == "value" {
            return value
                .read_scalar()
                .ok()
                .and_then(|scalar| scalar.to_f64().ok());
        }
    }
    None
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
    #[serde(default)]
    production_method_groups: Vec<String>,
    /// Paradox `required_construction` — literal points or a script-value name.
    #[serde(default, deserialize_with = "deserialize_opt_number_or_ident")]
    required_construction: Option<NumberOrIdent>,
}

#[derive(Debug, Clone)]
enum NumberOrIdent {
    Number(f64),
    Ident(String),
}

fn deserialize_opt_number_or_ident<'de, D>(
    deserializer: D,
) -> Result<Option<NumberOrIdent>, D::Error>
where
    D: Deserializer<'de>,
{
    struct OptNumberOrIdentVisitor;

    impl<'de> Visitor<'de> for OptNumberOrIdentVisitor {
        type Value = Option<NumberOrIdent>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a number, script-value ident, or null")
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D2: Deserializer<'de>>(
            self,
            deserializer: D2,
        ) -> Result<Self::Value, D2::Error> {
            deserializer.deserialize_any(NumberOrIdentVisitor).map(Some)
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            Ok(Some(NumberOrIdent::Number(v as f64)))
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            Ok(Some(NumberOrIdent::Number(v as f64)))
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
            Ok(Some(NumberOrIdent::Number(v)))
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            Ok(Some(NumberOrIdent::Ident(v.to_string())))
        }

        fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
            Ok(Some(NumberOrIdent::Ident(v)))
        }
    }

    struct NumberOrIdentVisitor;

    impl<'de> Visitor<'de> for NumberOrIdentVisitor {
        type Value = NumberOrIdent;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a number or script-value ident")
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            Ok(NumberOrIdent::Number(v as f64))
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            Ok(NumberOrIdent::Number(v as f64))
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
            Ok(NumberOrIdent::Number(v))
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            Ok(NumberOrIdent::Ident(v.to_string()))
        }

        fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
            Ok(NumberOrIdent::Ident(v))
        }
    }

    deserializer.deserialize_any(OptNumberOrIdentVisitor)
}

#[derive(Debug, Default, Deserialize)]
struct RawTechnology {
    /// Optional innovation cost (fixtures; vanilla often uses era costs).
    #[serde(default)]
    cost: Option<f64>,
    /// Paradox `unlocking_technologies` prerequisite list.
    #[serde(default)]
    unlocking_technologies: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawPmg {
    #[serde(default)]
    production_methods: Vec<String>,
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
