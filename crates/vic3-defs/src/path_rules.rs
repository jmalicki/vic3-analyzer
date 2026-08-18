//! Canonical browser/CLI allowlist for definition source paths.

use serde::{Deserialize, Serialize};

pub const COMMON_DIRS: &[&str] = &[
    "goods",
    "defines",
    "production_methods",
    "buildings",
    "building_groups",
    "pop_needs",
    "buy_packages",
    "cultures",
    "coat_of_arms",
    "flag_definitions",
    "named_colors",
    "pop_types",
];

/// Prefix of every interface-icon path. Only [`ICON_LEAFS`] under it are walked.
const ICONS_PREFIX: &[&str] = &["gfx", "interface", "icons"];
/// Leaf folders under `gfx/interface/icons` that we read `.dds` from.
/// `gfx/interface/icons` itself is not fully opened — country_icons and the rest prune.
pub(crate) const ICON_LEAFS: &[&str] = &[
    "goods_icons",
    "building_icons",
    "production_methods_icons",
    "production_method_icons",
    "production_method_groups_icons",
    "popup_icon",
    "alert_icons",
    "pops",
    "pops_icons",
    "pop_types",
    "pop_types_icons",
    "unit_types",
    "battalions",
    "military_unit_icons",
    "military_icons",
    "ships",
    "mobilization_options",
    "generic_icons",
    "state_status_icons",
    "notification_icons",
];
const COA_GFX_DIR: &[&str] = &["gfx", "coat_of_arms"];
const COA_GFX_LEAFS: &[&str] = &["patterns", "colored_emblems", "textured_emblems"];
pub(crate) const LOCALIZATION_PREFIXES: &[&str] = &[
    "goods_l_",
    "countries_l_",
    "buildings_l_",
    "building_groups_l_",
    "pop_types_l_",
    "cultures_l_",
    "state_regions_l_",
    "production_methods_l_",
    "production_method_groups_l_",
    "pop_needs_l_",
    "military_units_l_",
    "unit_types_l_",
    "alerts_l_",
];

const PRUNED_DIRS: &[&str] = &[
    "sound",
    "music",
    "map_data",
    "events",
    "history",
    "content_source",
    "dlc",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefsPathClass {
    /// Read this file into the definitions manifest.
    Read,
    /// Ignore this file, but continue the surrounding walk.
    Skip,
    /// Enumerate this directory.
    Descend,
    /// Do not enumerate this directory or any children.
    Prune,
}

/// Classify a game-relative or arbitrarily rooted path.
///
/// Rust owns this list. Browser walkers call the wasm export before reading a
/// file, while `load_from_files` applies it again as the trust boundary.
pub fn classify_defs_path(path: &str, is_directory: bool) -> DefsPathClass {
    let normalized = path.replace('\\', "/").to_lowercase();
    let segments = normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let name = segments.last().copied().unwrap_or_default();

    if !is_directory {
        if segments.contains(&"gfx") {
            let inside_icons = icon_route(&segments)
                .is_some_and(|index| segments.len() > index + ICONS_PREFIX.len());
            let inside_coa = coa_gfx_route(&segments).is_some_and(|index| {
                segments.len() > index + COA_GFX_DIR.len()
                    && segments
                        .get(index + COA_GFX_DIR.len())
                        .is_some_and(|leaf| COA_GFX_LEAFS.contains(leaf))
            });
            let readable = (inside_icons && name.ends_with(".dds"))
                || (inside_coa && (name.ends_with(".dds") || name.ends_with(".tga")));
            return if readable {
                DefsPathClass::Read
            } else {
                DefsPathClass::Skip
            };
        }
        let common = segments.iter().position(|segment| *segment == "common");
        let supported_common = common.is_some_and(|index| {
            segments
                .get(index + 1)
                .is_some_and(|dir| COMMON_DIRS.contains(dir))
                && name.ends_with(".txt")
        });
        let localization = segments
            .iter()
            .position(|segment| *segment == "localization");
        let supported_label = localization.is_some_and(|index| {
            segments.get(index + 1) == Some(&"english")
                && LOCALIZATION_PREFIXES
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
                && name.contains("_english")
                && name.ends_with(".yml")
        });
        return if supported_common || supported_label {
            DefsPathClass::Read
        } else {
            DefsPathClass::Skip
        };
    }

    if segments.contains(&"gfx") {
        return if icon_route(&segments).is_some() || coa_gfx_route(&segments).is_some() {
            DefsPathClass::Descend
        } else {
            DefsPathClass::Prune
        };
    }
    if PRUNED_DIRS.contains(&name) {
        return DefsPathClass::Prune;
    }
    if let Some(index) = segments.iter().position(|segment| *segment == "common") {
        return match segments.get(index + 1) {
            None => DefsPathClass::Descend,
            Some(dir) if COMMON_DIRS.contains(dir) => DefsPathClass::Descend,
            Some(_) => DefsPathClass::Prune,
        };
    }
    if let Some(index) = segments
        .iter()
        .position(|segment| *segment == "localization")
    {
        return match segments.get(index + 1) {
            None => DefsPathClass::Descend,
            Some(&"english") => DefsPathClass::Descend,
            Some(_) => DefsPathClass::Prune,
        };
    }
    if let Some(index) = segments.iter().position(|segment| *segment == "game") {
        return match segments.get(index + 1) {
            None | Some(&"common") | Some(&"localization") => DefsPathClass::Descend,
            Some(_) => DefsPathClass::Prune,
        };
    }
    DefsPathClass::Descend
}

/// Index of the `gfx` segment, but only while the path still leads to an
/// allowed icon leaf. Any other branch of `gfx` returns `None` so it can be pruned.
fn icon_route(segments: &[&str]) -> Option<usize> {
    let index = segments.iter().position(|segment| *segment == "gfx")?;
    let tail = &segments[index..];
    if tail.len() < ICONS_PREFIX.len() {
        return tail
            .iter()
            .zip(ICONS_PREFIX)
            .all(|(segment, expected)| segment == expected)
            .then_some(index);
    }
    if !tail
        .iter()
        .zip(ICONS_PREFIX)
        .all(|(segment, expected)| segment == expected)
    {
        return None;
    }
    match tail.get(ICONS_PREFIX.len()) {
        None => Some(index),
        Some(leaf) if ICON_LEAFS.contains(leaf) => Some(index),
        Some(_) => None,
    }
}

/// Namespace for [`crate::GameDefs::extra_icons`], or `None` for goods icons
/// (those stay keyed by good id in [`crate::GameDefs::icons`]).
pub(crate) fn extra_icon_kind(path: &str) -> Option<&'static str> {
    let normalized = path.replace('\\', "/").to_lowercase();
    let segments = normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let icons = segments.iter().position(|segment| *segment == "icons")?;
    let leaf = segments.get(icons + 1).copied()?;
    match leaf {
        "goods_icons" => None,
        "building_icons" => Some("building"),
        "production_methods_icons"
        | "production_method_icons"
        | "production_method_groups_icons" => Some("pm"),
        "popup_icon" | "alert_icons" => Some("alert"),
        "pops" | "pops_icons" | "pop_types" | "pop_types_icons" => Some("pop"),
        "unit_types"
        | "battalions"
        | "military_unit_icons"
        | "military_icons"
        | "ships"
        | "mobilization_options" => Some("military"),
        "generic_icons" | "state_status_icons" | "notification_icons" => Some("generic"),
        _ => None,
    }
}

fn coa_gfx_route(segments: &[&str]) -> Option<usize> {
    let index = segments.iter().position(|segment| *segment == "gfx")?;
    let ok = segments[index..]
        .iter()
        .zip(COA_GFX_DIR)
        .all(|(segment, expected)| segment == expected);
    if !ok {
        return None;
    }
    // Descend into gfx/coat_of_arms and its leaf folders only.
    let rest = &segments[index + COA_GFX_DIR.len()..];
    match rest.first() {
        None => Some(index),
        Some(leaf) if COA_GFX_LEAFS.contains(leaf) => Some(index),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_supported_files_from_any_root() {
        assert_eq!(
            classify_defs_path("Victoria 3/game/common/goods/00_goods.txt", false),
            DefsPathClass::Read
        );
        assert_eq!(
            classify_defs_path("game/localization/english/goods_l_english.yml", false),
            DefsPathClass::Read
        );
        assert_eq!(
            classify_defs_path("game/localization/english/countries_l_english.yml", false),
            DefsPathClass::Read
        );
        assert_eq!(
            classify_defs_path(
                "game/common/flag_definitions/00_flag_definitions.txt",
                false
            ),
            DefsPathClass::Read
        );
        assert_eq!(
            classify_defs_path("game/common/buildings/00_buildings.txt", false),
            DefsPathClass::Read
        );
        assert_eq!(
            classify_defs_path("game/common/building_groups/00_building_groups.txt", false),
            DefsPathClass::Read
        );
        assert_eq!(
            classify_defs_path("game/common/pop_types/00_pop_types.txt", false),
            DefsPathClass::Read
        );
        assert_eq!(
            classify_defs_path(
                "game/localization/english/production_methods_l_english.yml",
                false
            ),
            DefsPathClass::Read
        );
        assert_eq!(
            classify_defs_path("game/localization/french/goods_l_french.yml", false),
            DefsPathClass::Skip
        );
    }

    #[test]
    fn walks_gfx_only_as_far_as_allowed_icon_leafs() {
        assert_eq!(classify_defs_path("game/gfx", true), DefsPathClass::Descend);
        assert_eq!(
            classify_defs_path("game/gfx/interface/icons/goods_icons", true),
            DefsPathClass::Descend
        );
        assert_eq!(
            classify_defs_path("game/gfx/interface/icons/goods_icons/grain.dds", false),
            DefsPathClass::Read
        );
        assert_eq!(
            classify_defs_path("game/gfx/interface/icons/building_icons", true),
            DefsPathClass::Descend
        );
        assert_eq!(
            classify_defs_path(
                "game/gfx/interface/icons/building_icons/building_rye_farm.dds",
                false
            ),
            DefsPathClass::Read
        );
        assert_eq!(
            classify_defs_path("game/gfx/interface/icons/pops_icons", true),
            DefsPathClass::Descend
        );
        assert_eq!(
            classify_defs_path("game/gfx/interface/icons/pops_icons/academics.dds", false),
            DefsPathClass::Read
        );
        assert_eq!(
            extra_icon_kind("game/gfx/interface/icons/pops_icons/academics.dds"),
            Some("pop")
        );
        assert_eq!(
            classify_defs_path(
                "game/gfx/interface/icons/ships/ship_types/silhouette_frigate.dds",
                false
            ),
            DefsPathClass::Read
        );
        assert_eq!(
            extra_icon_kind("game/gfx/interface/icons/ships/ship_types/silhouette_frigate.dds"),
            Some("military")
        );
        assert_eq!(
            classify_defs_path(
                "game/gfx/interface/icons/mobilization_options/chocolate.dds",
                false
            ),
            DefsPathClass::Read
        );
        assert_eq!(
            extra_icon_kind("game/gfx/interface/icons/generic_icons/population.dds"),
            Some("generic")
        );
        assert_eq!(
            classify_defs_path("game/gfx/models", true),
            DefsPathClass::Prune
        );
        assert_eq!(
            classify_defs_path("game/gfx/interface/icons/country_icons", true),
            DefsPathClass::Prune
        );
        assert_eq!(
            classify_defs_path("game/gfx/interface/icons/country_icons/flag.dds", false),
            DefsPathClass::Skip
        );
        assert_eq!(
            classify_defs_path("game/gfx/coat_of_arms/patterns", true),
            DefsPathClass::Descend
        );
        assert_eq!(
            classify_defs_path("game/gfx/coat_of_arms/patterns/pattern_solid.tga", false),
            DefsPathClass::Read
        );
        // The folder itself is on the route, but it is not a file inside it.
        assert_eq!(
            classify_defs_path("game/gfx/interface/icons/goods_icons", false),
            DefsPathClass::Skip
        );
    }

    #[test]
    fn prunes_large_or_unrelated_subtrees() {
        assert_eq!(classify_defs_path("game/sound", true), DefsPathClass::Prune);
        assert_eq!(
            classify_defs_path("game/common/laws", true),
            DefsPathClass::Prune
        );
        assert_eq!(
            classify_defs_path("game/common/goods", true),
            DefsPathClass::Descend
        );
    }
}
