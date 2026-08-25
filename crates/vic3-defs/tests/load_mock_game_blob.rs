//! Guard the committed mock_game postcard against GameDefs layout drift.
//!
//! Regenerate when the postcard layout changes:
//! `cargo run -q -p vic3-defs --bin emit_fixture_blob -- tests/fixtures/mock_game.defs.postcard tests/fixtures/mock_game`
//! Bump [`vic3_defs::BLOB_VERSION`] when [`vic3_defs::GameDefs`] is not backward compatible.

use std::path::PathBuf;
use vic3_defs::{decode_blob, encode_blob, load_from_path, BLOB_VERSION};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn mock_game_root() -> PathBuf {
    repo_root().join("tests/fixtures/mock_game")
}

fn committed_blob_path() -> PathBuf {
    repo_root().join("tests/fixtures/mock_game.defs.postcard")
}

#[test]
fn committed_mock_game_blob_matches_fresh_encode() {
    let defs = load_from_path(mock_game_root()).expect("mock_game should load");
    let fresh = encode_blob(&defs).expect("encode fresh blob");

    let committed = std::fs::read(committed_blob_path()).expect(
        "tests/fixtures/mock_game.defs.postcard missing — regenerate with emit_fixture_blob",
    );

    let (version, _) = postcard::take_from_bytes::<u32>(&committed)
        .expect("committed blob must start with a postcard u32 version");
    assert_eq!(
        version, BLOB_VERSION,
        "committed mock_game postcard version must match BLOB_VERSION"
    );

    decode_blob(&committed)
        .expect("committed mock_game postcard must decode with current GameDefs");
    assert_eq!(
        committed, fresh,
        "committed mock_game.defs.postcard drifted from encode_blob(load_from_path(mock_game)); regenerate it"
    );
}
