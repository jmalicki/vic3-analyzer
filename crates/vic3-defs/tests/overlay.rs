//! File overlay merge: JSON overrides blob/install values; missing overlay leaves defs unchanged.

use std::fs;
use std::path::PathBuf;

use vic3_defs::{apply_overlay, load_from_path, load_overlay_json};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn overlay_path(name: &str) -> PathBuf {
    fixture_root().join("overlays").join(name)
}

#[test]
fn overlay_overrides_fixture_building_required_construction() {
    let mut defs = load_from_path(fixture_root()).expect("fixture defs");
    assert_eq!(
        defs.building_types["building_rye_farm"].required_construction,
        Some(200.0)
    );

    let json = fs::read_to_string(overlay_path("rye_farm_cost.json")).expect("overlay fixture");
    let overlay = load_overlay_json(&json).expect("parse overlay");
    apply_overlay(&mut defs, &overlay);

    assert_eq!(
        defs.building_types["building_rye_farm"].required_construction,
        Some(999.0)
    );
    // Unmentioned buildings stay at install values.
    assert_eq!(
        defs.building_types["building_coal_mine"].required_construction,
        Some(300.0)
    );
}

#[test]
fn without_overlay_fixture_required_construction_unchanged() {
    let defs = load_from_path(fixture_root()).expect("fixture defs");
    assert_eq!(
        defs.building_types["building_rye_farm"].required_construction,
        Some(200.0)
    );
    assert_eq!(
        defs.building_types["building_coal_mine"].required_construction,
        Some(300.0)
    );
}
