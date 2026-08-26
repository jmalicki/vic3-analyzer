//! Dense building-type indices aligned with [`crate::GameDefs::building_types_order`].
//!
//! Prefer [`BuildingTypeIdx`] over script strings in world / planning hot paths.
//! Resolve strings at load / API boundaries via [`crate::GameDefs::building_index_of`].

use std::fmt;

use serde::{Deserialize, Serialize};

/// Index into [`crate::GameDefs::building_types_order`].
///
/// Prefer this over raw `usize` or script ids so building types cannot be
/// confused with instance ids or other dense tables.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BuildingTypeIdx(u16);

impl BuildingTypeIdx {
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

impl fmt::Debug for BuildingTypeIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BuildingTypeIdx({})", self.0)
    }
}

impl fmt::Display for BuildingTypeIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
