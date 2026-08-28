//! Dense building-type indices aligned with [`crate::GameDefs::building_types_order`].
//!
//! Prefer [`BuildingTypeId`] over script strings in world / planning hot paths.
//! Resolve strings at load / API boundaries via [`crate::GameDefs::building_index_of`]
//! or [`crate::GameDefs::resolve_building_type_index`] when aliases may apply.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Index into [`crate::GameDefs::building_types_order`].
///
/// Prefer this over raw `usize` or script ids so building types cannot be
/// confused with instance ids or other dense tables.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BuildingTypeId(u16);

impl BuildingTypeId {
    /// Construct from a position when it fits the compact representation.
    #[inline]
    pub fn try_from_usize(index: usize) -> Option<Self> {
        u16::try_from(index).ok().map(Self)
    }

    /// Construct from a position in `building_types_order`.
    ///
    /// # Panics
    ///
    /// Panics if `index` exceeds `u16::MAX` (far beyond any vanilla building table).
    #[inline]
    pub fn from_usize(index: usize) -> Self {
        Self::try_from_usize(index).expect("building type count fits in u16")
    }

    /// Position in `building_types_order`.
    #[inline]
    pub fn as_usize(self) -> usize {
        usize::from(self.0)
    }

    /// Raw discriminant for tests, SQL, and sparse tables.
    #[inline]
    pub fn raw(self) -> u16 {
        self.0
    }
}

impl fmt::Debug for BuildingTypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BuildingTypeId({})", self.0)
    }
}

impl fmt::Display for BuildingTypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Alternate script ids for the same building type (Paradox / mod spelling drift).
pub const BUILDING_TYPE_ALIASES: &[(&str, &str)] = &[
    ("building_shipyard", "building_shipyards"),
    ("building_shipyards", "building_shipyard"),
    ("building_barrack", "building_barracks"),
    ("building_barracks", "building_barrack"),
];

/// Return the alternate script id for `building_type`, if one is known.
pub fn building_type_alias(building_type: &str) -> Option<&'static str> {
    BUILDING_TYPE_ALIASES
        .iter()
        .find_map(|(from, to)| (building_type == *from).then_some(*to))
}

#[cfg(test)]
mod tests {
    use crate::GameDefs;

    #[test]
    fn resolve_building_type_index_accepts_known_aliases() {
        let mut defs = GameDefs::default();
        defs.ensure_building_type("building_shipyard");
        assert_eq!(
            defs.resolve_building_type_index("building_shipyards"),
            defs.resolve_building_type_index("building_shipyard"),
        );

        let mut defs = GameDefs::default();
        defs.ensure_building_type("building_barracks");
        assert_eq!(
            defs.resolve_building_type_index("building_barrack"),
            defs.resolve_building_type_index("building_barracks"),
        );
    }
}
