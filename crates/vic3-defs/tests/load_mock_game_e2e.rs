//! Ensures the PR #76 mock_game tree still parses for e2e.

use std::path::PathBuf;
use vic3_defs::{encode_blob, load_from_path};

fn mock_game_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/mock_game")
}

#[test]
fn mock_game_fixture_loads_and_encodes() {
    let defs = load_from_path(mock_game_root()).expect("mock_game should load");
    assert!(defs.goods.len() >= 3, "expected mock goods");
    let blob = encode_blob(&defs).expect("encode");
    assert!(!blob.is_empty());
}
