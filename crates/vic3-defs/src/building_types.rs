//! Dense building-type indices aligned with [`crate::GameDefs::building_types_order`].
//!
//! Prefer [`BuildingTypeId`] over script strings in world / planning hot paths.
//! Resolve strings at load / API boundaries via [`crate::GameDefs::building_index_of`]
//! or [`crate::GameDefs::resolve_building_type_index`] when aliases may apply.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, OnceLock};

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

/// Derived script id → [`BuildingTypeId`] map over
/// [`crate::GameDefs::building_types_order`].
///
/// Without it every [`crate::GameDefs::building_index_of`] call scans the whole
/// order table, which is ~200 string comparisons on a real install and showed up
/// as 6% of planner CPU.
///
/// The map is built on first lookup, so no construction path —
/// [`crate::load_from_path`], [`crate::DefsBuilder`], [`crate::decode_blob`], or
/// a struct literal — has to remember to populate it.
///
/// `building_types_order` is a public field, so it can also be edited after the
/// map was built. The map is therefore never taken at its word: a hit counts
/// only while it still names the same building type, and a miss falls back to
/// the scan, because a miss cannot distinguish "absent" from "the order changed
/// under us". A stale map can only cost a scan, never return a wrong index. The
/// `&mut GameDefs` helpers that edit the order drop the map to keep misses O(1).
///
/// [`OnceLock`] rather than a lock: this is written once and read constantly, and
/// [`crate::GameDefs`] has to stay `Sync` for the `Arc<GameDefs>` that
/// `vic3-sql` hands to DataFusion providers.
///
/// Derived data: skipped by serde, cloned by handle, and always equal.
#[derive(Debug, Default, Clone)]
pub struct BuildingTypeIndex(OnceLock<Arc<HashMap<String, BuildingTypeId>>>);

impl BuildingTypeIndex {
    /// Position of `building_type` in `order`.
    ///
    /// Equivalent to `order.iter().position(|id| id == building_type)`, and O(1)
    /// whenever the map is current.
    pub(crate) fn position(&self, order: &[String], building_type: &str) -> Option<BuildingTypeId> {
        let found = match self
            .0
            .get_or_init(|| Arc::new(build(order)))
            .get(building_type)
        {
            // Trust a hit only while it still names the same building type.
            Some(&id)
                if order
                    .get(id.as_usize())
                    .is_some_and(|name| name == building_type) =>
            {
                Some(id)
            }
            // A miss means either the id is absent or `building_types_order` was
            // edited after the map was built. Only a scan tells them apart, so
            // never report a miss on the map's word alone.
            _ => scan(order, building_type),
        };
        debug_assert_eq!(
            found,
            scan(order, building_type),
            "building type index disagrees with building_types_order"
        );
        found
    }

    /// Drop the map so the next lookup rebuilds it.
    ///
    /// Purely an optimization — lookups stay correct against a stale map — but
    /// it keeps misses off the scan path after the order changes.
    pub(crate) fn clear(&mut self) {
        self.0.take();
    }
}

fn build(order: &[String]) -> HashMap<String, BuildingTypeId> {
    let mut by_script_id = HashMap::with_capacity(order.len());
    for (position, id) in order.iter().enumerate() {
        // First one wins, matching the `position()` scan this replaces.
        by_script_id
            .entry(id.clone())
            .or_insert_with(|| BuildingTypeId::from_usize(position));
    }
    by_script_id
}

fn scan(order: &[String], building_type: &str) -> Option<BuildingTypeId> {
    order
        .iter()
        .position(|id| id == building_type)
        .map(BuildingTypeId::from_usize)
}

impl PartialEq for BuildingTypeIndex {
    /// Always equal: the map is derived from `building_types_order`, which the
    /// derived [`crate::GameDefs`] comparison already covers.
    fn eq(&self, _other: &Self) -> bool {
        true
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
    use super::BuildingTypeId;
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

    #[test]
    fn aliases_stay_equivalent_and_unknown_ids_stay_unknown() {
        let mut defs = GameDefs::default();
        defs.ensure_building_type("building_wheat_farm");
        defs.ensure_building_type("building_shipyard");

        assert!(defs.building_types_equivalent("building_shipyard", "building_shipyards"));
        assert!(defs.building_types_equivalent("building_shipyards", "building_shipyard"));
        assert!(!defs.building_types_equivalent("building_shipyard", "building_wheat_farm"));
        assert_eq!(defs.building_index_of("building_shipyards"), None);
        assert_eq!(defs.resolve_building_type_index("building_nope"), None);
        assert_eq!(
            defs.canonical_building_type_key("building_shipyards")
                .as_deref(),
            Some("building_shipyard"),
        );
    }

    /// Every id must map to its own slot in `building_types_order`, whichever
    /// order the ids arrive in.
    #[test]
    fn index_agrees_with_order_after_the_order_grows() {
        let mut defs = GameDefs::default();
        for (expected, id) in ["building_a", "building_b", "building_c"]
            .into_iter()
            .enumerate()
        {
            // Look up before and after each append so a stale map would show.
            assert_eq!(defs.building_index_of(id), None);
            defs.ensure_building_type(id);
            assert_eq!(
                defs.building_index_of(id),
                Some(BuildingTypeId::from_usize(expected)),
            );
        }

        for (position, id) in defs.building_types_order.clone().iter().enumerate() {
            assert_eq!(
                defs.building_index_of(id),
                Some(BuildingTypeId::from_usize(position)),
            );
            assert_eq!(
                defs.building_by_index(BuildingTypeId::from_usize(position)),
                id
            );
        }
    }

    /// `rebuild_building_types_order` can reorder ids without changing the
    /// length, which the index cannot notice on its own.
    #[test]
    fn index_follows_a_reordered_order_table() {
        let mut defs = GameDefs::default();
        defs.ensure_building_type("building_zzz");
        defs.ensure_building_type("building_aaa");
        assert_eq!(
            defs.building_index_of("building_zzz"),
            Some(BuildingTypeId::from_usize(0)),
        );

        // Sorts by map key, so `building_aaa` moves to slot 0.
        defs.rebuild_building_types_order();

        assert_eq!(
            defs.building_index_of("building_aaa"),
            Some(BuildingTypeId::from_usize(0)),
        );
        assert_eq!(
            defs.building_index_of("building_zzz"),
            Some(BuildingTypeId::from_usize(1)),
        );
    }

    /// `building_types_order` is public, so an entry can be swapped for a new
    /// script id without changing the length — invisible to the built map.
    /// The replacement still has to resolve, and the id it replaced must not.
    #[test]
    fn index_follows_a_same_length_edit_to_the_public_order() {
        let mut defs = GameDefs::default();
        defs.ensure_building_type("building_a");
        defs.ensure_building_type("building_b");
        // Build the map against the original order.
        assert_eq!(
            defs.building_index_of("building_b"),
            Some(BuildingTypeId::from_usize(1)),
        );

        defs.building_types_order[1] = "building_c".into();

        assert_eq!(
            defs.building_index_of("building_c"),
            Some(BuildingTypeId::from_usize(1)),
        );
        assert_eq!(defs.building_index_of("building_b"), None);
        assert_eq!(
            defs.building_index_of("building_a"),
            Some(BuildingTypeId::from_usize(0)),
        );
    }

    /// A clone shares the built map; both sides must still answer correctly.
    #[test]
    fn cloned_defs_resolve_independently() {
        let mut defs = GameDefs::default();
        defs.ensure_building_type("building_wheat_farm");
        assert_eq!(
            defs.building_index_of("building_wheat_farm"),
            Some(BuildingTypeId::from_usize(0)),
        );

        let mut clone = defs.clone();
        clone.ensure_building_type("building_shipyard");

        assert_eq!(
            clone.building_index_of("building_shipyard"),
            Some(BuildingTypeId::from_usize(1)),
        );
        assert_eq!(defs.building_index_of("building_shipyard"), None);
        assert_eq!(
            defs.building_index_of("building_wheat_farm"),
            Some(BuildingTypeId::from_usize(0)),
        );
    }

    /// Derived data must not affect equality or the serialized form.
    #[test]
    fn the_index_does_not_change_equality() {
        let mut left = GameDefs::default();
        left.ensure_building_type("building_wheat_farm");
        let right = left.clone();
        // Build the map on one side only.
        assert!(left.building_index_of("building_wheat_farm").is_some());

        assert_eq!(left, right);
    }
}
