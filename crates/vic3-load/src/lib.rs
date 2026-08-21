//! Load Victoria 3 `.v3` saves into an owned serde IR.
//!
//! # Role in the pipeline
//!
//! ```text
//! save (+ tokens) → vic3-load → IR
//! install / fixture / blob → vic3-defs → GameDefs
//! IR + GameDefs → vic3-prices → … → plan → vic3-api
//! ```
//!
//! Envelope detection (zip / plaintext / binary), Vic3 binary flavor, and melt
//! come from pdx-tools [`vic3save`] + [jomini]. This crate owns the deserialize
//! targets so fields can grow past upstream `Vic3Save` without forking the
//! parser. Downstream crates project from [`WorldSnapshot`]; they do not parse
//! Clausewitz themselves.
//!
//! Architecture overview: [`docs/architecture.md`](../../../docs/architecture.md).
//!
//! # Token maps
//!
//! Binary (ironman) saves need a **user-supplied** Paradox token map
//! ([`BasicTokenResolver`]). Text saves do not. Tokens are never redistributed;
//! load them with [`load_tokens_slice`] / [`load_tokens_path`] (typically from
//! `VIC3_TOKENS`). An empty resolver on a binary save returns
//! [`LoadError::MissingTokens`] before serde runs.
//!
//! # Deserialize targets
//!
//! - [`Save`] — full file-shaped IR (markets, trade routes, construction,
//!   military). Used by `parse_save` summaries and tests.
//! - [`WorldSave`] — managers the price + planning projections read. Markets
//!   and trade routes are skipped at deserialize time; technology, construction
//!   queues, interest markers, and army formations are kept.
//!
//! Prefer [`load_slice_world`] / [`load_path_world`] on the prices path. Prefer
//! [`Save`] when market / trade-route counts are part of the answer.
//!
//! Ironman `.v3` files are typically **one** zip member (`gamestate`). Skipping
//! unknown keys still inflates that blob; [`WorldSave`] only avoids building
//! unused maps.
//!
//! # Sparse Paradox ids
//!
//! Manager databases use sparse `u32` keys with deleted slots as the identifier
//! `none` ([`Manager`]). Culture / goods indices in binary saves are integers;
//! plaintext fixtures often use script ids — the IR deserializers accept both.
//!
//! # Patch-export vs IR round-trip
//!
//! [`export_save`] rewrites `building_manager.database` entries in the
//! **original uncompressed plaintext**. It does **not** serialize the serde IR
//! back to Clausewitz (lossy). Ironman / binary envelopes are rejected. See the
//! patch-export section of
//! [`docs/architecture.md`](../../../docs/architecture.md).
//!
//! [jomini]: https://docs.rs/jomini
//! [`vic3save`]: https://github.com/pdx-tools/pdx-tools

mod error;
mod export;
mod ir;
mod maybe;

use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use jomini::binary::TokenResolver;
use vic3save::{DeserializeVic3, ReaderAt, Vic3File};

pub use error::LoadError;
pub use export::{export_save, ExportError, ExtraLevelsPatch, ProductionMethodPatch, SavePatch};
pub use ir::{
    all_constructions, army_power_projection_for, constructions_for, declared_interest_for,
    hydrate_country_techs, navy_power_projection_for, normalize_interest_ids, queued_building_for,
    queued_tech_for, researched_techs_for, Budget, Building, BuildingGoods, ConstructionOrder,
    ConstructionQueueEntry, ConstructionQueueKind, Country, Culture, DeclaredInterest, IndexQtyMap,
    InterestMarker, Manager, Market, Meta, MilitaryFormation, MilitaryHq, MilitaryUnit,
    MobilizationEntry, Player, Pop, Save, State, StatePopStatistics, TechnologyEntry, TradeRoute,
    WorldSave, WorldSnapshot,
};
pub use vic3save::{BasicTokenResolver, Vic3Date};

/// Crate version from Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Empty token resolver for plaintext saves.
///
/// # Panics
///
/// Never in practice: empty token text has no lines to parse.
pub fn empty_tokens() -> BasicTokenResolver {
    BasicTokenResolver::from_text_lines(&b""[..]).expect("empty token text has no lines to parse")
}

/// Parse a Paradox token map (`0xabcd field_name` per line).
///
/// # Errors
///
/// Returns [`LoadError::Tokens`] when the text is not a valid token map.
pub fn load_tokens_slice(data: &[u8]) -> Result<BasicTokenResolver, LoadError> {
    BasicTokenResolver::from_text_lines(data).map_err(|err| LoadError::Tokens(err.to_string()))
}

/// Load a token map from a filesystem path (typically `VIC3_TOKENS`).
///
/// # Errors
///
/// Returns [`LoadError::Io`] on open failure, or [`LoadError::Tokens`] on parse
/// failure.
pub fn load_tokens_path(path: impl AsRef<Path>) -> Result<BasicTokenResolver, LoadError> {
    let file = File::open(path.as_ref())?;
    load_tokens_reader(BufReader::new(file))
}

/// Load a token map from any [`BufRead`] source.
///
/// # Errors
///
/// Returns [`LoadError::Tokens`] when the text is not a valid token map.
pub fn load_tokens_reader(reader: impl BufRead) -> Result<BasicTokenResolver, LoadError> {
    BasicTokenResolver::from_text_lines(reader).map_err(|err| LoadError::Tokens(err.to_string()))
}

/// Load a `.v3` (zip, uncompressed text, or binary) from bytes into [`Save`].
///
/// Pass [`empty_tokens`] for plaintext. After deserialize, researched techs are
/// hydrated onto each [`Country`] via [`Save::hydrate_country_techs`].
///
/// # Errors
///
/// - [`LoadError::MissingTokens`] — binary save with an empty resolver
/// - [`LoadError::Vic3`] — envelope / melt / deserialize failure
///
/// # Examples
///
/// ```
/// use vic3_load::{empty_tokens, load_slice};
///
/// let bytes = std::fs::read(concat!(
///     env!("CARGO_MANIFEST_DIR"),
///     "/tests/fixtures/plaintext.txt"
/// ))
/// .expect("fixture");
/// let save = load_slice(&bytes, empty_tokens()).expect("plaintext");
/// assert_eq!(save.meta_data.version, "1.9.0");
/// ```
pub fn load_slice(data: &[u8], tokens: impl TokenResolver) -> Result<Save, LoadError> {
    let mut save: Save = load_slice_as(data, tokens)?;
    save.hydrate_country_techs();
    Ok(save)
}

/// Load only the managers [`WorldSnapshot`] needs into [`WorldSave`].
///
/// Prefer this on the prices / `World::from_save` path. [`load_slice`] still
/// exists for summaries that count markets and trade routes.
///
/// # Errors
///
/// Same as [`load_slice`].
pub fn load_slice_world(data: &[u8], tokens: impl TokenResolver) -> Result<WorldSave, LoadError> {
    let mut save: WorldSave = load_slice_as(data, tokens)?;
    save.hydrate_country_techs();
    Ok(save)
}

/// Load a `.v3` from a filesystem path (typically `VIC3_SAVE`) into [`Save`].
///
/// # Errors
///
/// [`LoadError::Io`] on open failure, plus the same deserialize errors as
/// [`load_slice`].
pub fn load_path(path: impl AsRef<Path>, tokens: impl TokenResolver) -> Result<Save, LoadError> {
    let mut save: Save = load_path_as(path, tokens)?;
    save.hydrate_country_techs();
    Ok(save)
}

/// [`load_path`] into [`WorldSave`]: skip market / trade-route IR; keep tech and
/// construction queues for planning projection.
///
/// # Errors
///
/// Same as [`load_path`].
pub fn load_path_world(
    path: impl AsRef<Path>,
    tokens: impl TokenResolver,
) -> Result<WorldSave, LoadError> {
    let mut save: WorldSave = load_path_as(path, tokens)?;
    save.hydrate_country_techs();
    Ok(save)
}

fn load_slice_as<T: serde::de::DeserializeOwned>(
    data: &[u8],
    tokens: impl TokenResolver,
) -> Result<T, LoadError> {
    let file = Vic3File::from_slice(data)?;
    deserialize_as(&file, tokens)
}

fn load_path_as<T: serde::de::DeserializeOwned>(
    path: impl AsRef<Path>,
    tokens: impl TokenResolver,
) -> Result<T, LoadError> {
    let file = File::open(path.as_ref())?;
    let file = Vic3File::from_file(file)?;
    deserialize_as(&file, tokens)
}

fn deserialize_as<T: serde::de::DeserializeOwned, R: ReaderAt>(
    file: &Vic3File<R>,
    tokens: impl TokenResolver,
) -> Result<T, LoadError> {
    if file.header().kind().is_binary() && tokens.is_empty() {
        return Err(LoadError::MissingTokens);
    }
    let mut view = file;
    Ok(view.deserialize(tokens)?)
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn version_is_semver() {
        assert!(!version().is_empty());
    }
}
