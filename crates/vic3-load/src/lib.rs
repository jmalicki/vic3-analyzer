//! Load Victoria 3 `.v3` saves into a typed IR.
//!
//! Envelope, zip/text/binary detection, and Vic3 binary flavor come from
//! pdx-tools `vic3save` + jomini. This crate owns the serde IR so it can grow
//! past upstream `Vic3Save`.
//!
//! Binary (ironman) saves need a **user-supplied** token map
//! ([`BasicTokenResolver`]). Text saves do not. Tokens are never redistributed.
//!
//! Two deserialize targets:
//! - [`Save`] — file-shaped; markets, trade routes, and construction queues
//!   included. Used by `parse_save` and tests.
//! - [`WorldSave`] — managers the price + planning projections read. Markets
//!   and trade routes are skipped; technology and construction queues are kept.
//!
//! Ironman `.v3` files are typically **one** zip member (`gamestate`). Skipping
//! unknown keys still inflates that blob; [`WorldSave`] only avoids building
//! unused maps. Interning inside jomini is not worth it while Pop IR stays
//! under ~0.1 s.

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
    hydrate_country_techs, queued_building_for, queued_tech_for, researched_techs_for, Budget,
    Building, BuildingGoods, ConstructionOrder, Country, Culture, IndexQtyMap, Manager, Market,
    Meta, MilitaryFormation, MilitaryHq, MilitaryUnit, MobilizationEntry, Player, Pop, Save, State,
    StatePopStatistics, TechnologyEntry, TradeRoute, WorldSave, WorldSnapshot,
};
pub use vic3save::{BasicTokenResolver, Vic3Date};

/// Crate version from Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Empty token resolver for plaintext saves.
pub fn empty_tokens() -> BasicTokenResolver {
    BasicTokenResolver::from_text_lines(&b""[..]).expect("empty token text has no lines to parse")
}

/// Parse a Paradox token map (`0xabcd field_name` per line).
pub fn load_tokens_slice(data: &[u8]) -> Result<BasicTokenResolver, LoadError> {
    BasicTokenResolver::from_text_lines(data).map_err(|err| LoadError::Tokens(err.to_string()))
}

/// Load a token map from a filesystem path (`VIC3_TOKENS`).
pub fn load_tokens_path(path: impl AsRef<Path>) -> Result<BasicTokenResolver, LoadError> {
    let file = File::open(path.as_ref())?;
    load_tokens_reader(BufReader::new(file))
}

/// Load a token map from any [`BufRead`] source.
pub fn load_tokens_reader(reader: impl BufRead) -> Result<BasicTokenResolver, LoadError> {
    BasicTokenResolver::from_text_lines(reader).map_err(|err| LoadError::Tokens(err.to_string()))
}

/// Load a `.v3` (zip, uncompressed text, or binary) from bytes.
///
/// Pass [`empty_tokens`] for plaintext. Binary saves with an empty resolver
/// return [`LoadError::MissingTokens`].
pub fn load_slice(data: &[u8], tokens: impl TokenResolver) -> Result<Save, LoadError> {
    let mut save: Save = load_slice_as(data, tokens)?;
    save.hydrate_country_techs();
    Ok(save)
}

/// Load only the managers [`WorldSnapshot`] needs.
///
/// Prefer this on the prices / `World::from_save` path. [`load_slice`] still
/// exists for summaries that count markets and trade routes.
pub fn load_slice_world(data: &[u8], tokens: impl TokenResolver) -> Result<WorldSave, LoadError> {
    let mut save: WorldSave = load_slice_as(data, tokens)?;
    save.hydrate_country_techs();
    Ok(save)
}

/// Load a `.v3` from a filesystem path (`VIC3_SAVE`).
pub fn load_path(path: impl AsRef<Path>, tokens: impl TokenResolver) -> Result<Save, LoadError> {
    let mut save: Save = load_path_as(path, tokens)?;
    save.hydrate_country_techs();
    Ok(save)
}

/// [`load_path`] into [`WorldSave`]: skip market / trade-route IR; keep tech and
/// construction queues for planning projection.
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
