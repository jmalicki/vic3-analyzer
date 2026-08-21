//! Player vs full-save row scope for fact tables (`docs/sql.md`).
//!
//! Short names (`states`, `active.states`, `latest.states`, …) are **player-owned
//! only** — same strict [`World::player_tag`](vic3_prices::World::player_tag) rule
//! as `alerts()`. Full-save twins are registered as `world_*` /
//! `active.world_*` / `latest.world_*`.

use std::collections::BTreeSet;

use vic3_prices::World;

/// Whether a fact-table scan returns player-owned rows or the full save.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableScope {
    /// Unqualified / `active.*` / `latest.*` short names — player-owned only.
    #[default]
    Player,
    /// `world_*` names — unfiltered save-wide rows.
    World,
}

impl TableScope {
    /// `true` when the provider should emit every row in the binding.
    #[inline]
    pub fn is_world(self) -> bool {
        matches!(self, Self::World)
    }

    /// `true` when the provider should filter to the played country / its states.
    #[inline]
    pub fn is_player(self) -> bool {
        matches!(self, Self::Player)
    }
}

/// State ids owned by [`World::player_tag`] (strict; no first-country fallback).
///
/// Mirrors `player_state_ids` in `vic3-prices` optimize, but only when
/// `player_tag` is set. [`None`] when there is no player tag or the tag does
/// not resolve to a countries row.
pub fn player_owned_state_ids(world: &World) -> Option<BTreeSet<u32>> {
    let tag = world.player_tag.as_deref()?;
    let country = world.country_by_tag(tag)?;
    let mut ids: BTreeSet<u32> = country.states.iter().copied().collect();
    for state in &world.states {
        if state.country == Some(country.id) {
            ids.insert(state.id);
        }
    }
    Some(ids)
}

/// Whether `state_id` is in player-owned scope (or scope is [`TableScope::World`]).
///
/// Rows with a missing `state_id` are excluded under player scope.
pub fn state_in_scope(scope: TableScope, world: &World, state_id: Option<u32>) -> bool {
    if scope.is_world() {
        return true;
    }
    let Some(id) = state_id else {
        return false;
    };
    player_owned_state_ids(world).is_some_and(|ids| ids.contains(&id))
}

/// Whether a country tag is the played country (or scope is world).
pub fn country_tag_in_scope(scope: TableScope, world: &World, tag: &str) -> bool {
    if scope.is_world() {
        return true;
    }
    world.player_tag.as_deref() == Some(tag)
}

/// Constructions: player country queue **or** player-owned state (or world).
pub fn construction_in_scope(
    scope: TableScope,
    world: &World,
    country_id: Option<u32>,
    state_id: Option<u32>,
) -> bool {
    if scope.is_world() {
        return true;
    }
    let Some(tag) = world.player_tag.as_deref() else {
        return false;
    };
    let Some(country) = world.country_by_tag(tag) else {
        return false;
    };
    if country_id == Some(country.id) {
        return true;
    }
    state_in_scope(scope, world, state_id)
}
