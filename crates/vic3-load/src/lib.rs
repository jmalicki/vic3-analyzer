//! Load Victoria 3 `.v3` saves into a typed IR.
//!
//! Envelope, zip/text/binary detection, and Vic3 binary flavor come from
//! pdx-tools `vic3save` + jomini. This crate owns the serde IR so it can grow
//! past upstream `Vic3Save`.
//!
//! Binary (ironman) saves need a **user-supplied** token map
//! ([`BasicTokenResolver`]). Text saves do not. Tokens are never redistributed.

mod error;
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
pub use ir::{
    Budget, Building, BuildingGoods, ConstructionOrder, Country, Culture, IndexQtyMap, Manager,
    Market, Meta, Player, Pop, Save, State, StatePopStatistics, TradeRoute,
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
    let file = Vic3File::from_slice(data)?;
    deserialize_save(&file, tokens)
}

/// Load a `.v3` from a filesystem path (`VIC3_SAVE`).
pub fn load_path(path: impl AsRef<Path>, tokens: impl TokenResolver) -> Result<Save, LoadError> {
    let file = File::open(path.as_ref())?;
    let file = Vic3File::from_file(file)?;
    deserialize_save(&file, tokens)
}

fn deserialize_save<R: ReaderAt>(
    file: &Vic3File<R>,
    tokens: impl TokenResolver,
) -> Result<Save, LoadError> {
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
