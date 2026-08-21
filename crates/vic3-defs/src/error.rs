use std::path::PathBuf;

/// Errors from loading game definitions or the wasm defs blob.
///
/// [`Self::NotAGameRoot`] covers both a bad install path and an in-memory
/// selection that never included `common/goods`.
#[derive(Debug, thiserror::Error)]
pub enum DefsError {
    #[error("I/O error while reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse Clausewitz text in {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: jomini::Error,
    },
    #[error("no Victoria 3 data root at {0} (expected `common/goods` or `game/common/goods`)")]
    NotAGameRoot(PathBuf),
    #[error("defs blob version {found} is not supported (expected {expected})")]
    BlobVersion { found: u32, expected: u32 },
    #[error("failed to encode defs blob: {0}")]
    BlobEncode(postcard::Error),
    #[error("failed to decode defs blob: {0}")]
    BlobDecode(postcard::Error),
    #[error("failed to parse defs overlay JSON: {0}")]
    OverlayJson(#[source] serde_json::Error),
}
