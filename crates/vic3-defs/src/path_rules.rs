//! Canonical browser/CLI allowlist for definition source paths.

use serde::{Deserialize, Serialize};

pub const COMMON_DIRS: &[&str] = &[
    "goods",
    "defines",
    "production_methods",
    "pop_needs",
    "buy_packages",
    "cultures",
];

/// The only route into `gfx` we walk: goods icons are small and needed for the
/// UI, while the rest of that tree is gigabytes of art.
const ICON_DIR: &[&str] = &["gfx", "interface", "icons", "goods_icons"];

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
            let inside_icons =
                icon_route(&segments).is_some_and(|index| segments.len() > index + ICON_DIR.len());
            return if inside_icons && name.ends_with(".dds") {
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
                && name.starts_with("goods_l_")
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
        return if icon_route(&segments).is_some() {
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

/// Index of the `gfx` segment, but only while the path still leads to the
/// goods icons. Any other branch of `gfx` returns `None` so it can be pruned.
fn icon_route(segments: &[&str]) -> Option<usize> {
    let index = segments.iter().position(|segment| *segment == "gfx")?;
    segments[index..]
        .iter()
        .zip(ICON_DIR)
        .all(|(segment, expected)| segment == expected)
        .then_some(index)
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
            classify_defs_path("game/localization/french/goods_l_french.yml", false),
            DefsPathClass::Skip
        );
    }

    #[test]
    fn walks_gfx_only_as_far_as_the_goods_icons() {
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
            classify_defs_path("game/gfx/models", true),
            DefsPathClass::Prune
        );
        assert_eq!(
            classify_defs_path("game/gfx/interface/icons/country_icons", true),
            DefsPathClass::Prune
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
