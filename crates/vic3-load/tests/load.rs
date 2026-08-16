//! Integration tests for `vic3-load`.

use std::path::PathBuf;
use vic3_load::{empty_tokens, load_path, load_slice, load_tokens_path, LoadError};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn plaintext_fixture_loads() {
    let save = load_path(fixture("plaintext.txt"), empty_tokens()).expect("plaintext fixture");

    assert_eq!(save.meta_data.version, "1.9.0");
    assert_eq!(
        save.meta_data.game_date,
        Some(vic3_load::Vic3Date::from_ymdh(1836, 1, 1, 0))
    );

    let ger = save.country_by_tag("GER").expect("GER in fixture");
    assert_eq!(ger.definition, "GER");
    assert_eq!(ger.infamy, Some(12.5));
    assert_eq!(ger.budget.treasury(), Some(10000.0));
    assert_eq!(ger.budget.credit, Some(500.0));

    let state = save.states.database.get(&1).and_then(|s| s.as_ref());
    assert_eq!(
        state.and_then(|s| s.region.as_deref()),
        Some("STATE_BRANDENBURG")
    );

    let farm = save
        .building_manager
        .iter_present()
        .find(|(_, b)| b.building == "building_rye_farm")
        .map(|(_, b)| b)
        .expect("rye farm");
    assert_eq!(farm.level, 2);
    assert_eq!(farm.state, Some(1));
    assert_eq!(farm.active_production_methods(), ["pm_simple_forestry"]);

    let pop = save
        .pops
        .iter_present()
        .next()
        .map(|(_, p)| p)
        .expect("pop");
    assert_eq!(pop.profession.as_deref(), Some("farmers"));
    assert_eq!(pop.size, Some(10000.0));
    assert_eq!(pop.wealth, Some(8));
    assert_eq!(pop.culture.as_deref(), Some("north_german"));
    assert_eq!(pop.state, Some(1));

    assert_eq!(save.market_manager.iter_present().count(), 1);
    let route = save
        .trade_route_manager
        .iter_present()
        .next()
        .map(|(_, r)| r)
        .expect("trade route");
    assert_eq!(route.goods.as_deref(), Some("grain"));
    assert_eq!(route.volume, Some(50.0));

    let order = save
        .building_constructions
        .iter_present()
        .next()
        .map(|(_, o)| o)
        .expect("construction order");
    assert_eq!(
        order.building.as_deref(),
        Some("building_construction_sector")
    );
}

/// Real saves list the active method of every PM group under the plural key.
/// Reading only the singular key leaves buildings with no goods flows at all,
/// which silently flattens every price to its base.
#[test]
fn plural_production_methods_are_read() {
    let text = std::fs::read_to_string(fixture("plaintext.txt"))
        .unwrap()
        .replace(
            "production_method=\"pm_simple_forestry\"",
            "production_methods={ \"pm_simple_forestry\" \"pm_no_automation\" }",
        );
    let save = load_slice(text.as_bytes(), empty_tokens()).expect("plural PM save");

    let farm = save
        .building_manager
        .iter_present()
        .find(|(_, b)| b.building == "building_rye_farm")
        .map(|(_, b)| b)
        .expect("rye farm");
    assert_eq!(
        farm.active_production_methods(),
        ["pm_simple_forestry", "pm_no_automation"]
    );
}

#[test]
fn database_none_vs_object() {
    let save = load_slice(
        &std::fs::read(fixture("plaintext.txt")).unwrap(),
        empty_tokens(),
    )
    .unwrap();

    assert_eq!(save.country_manager.database.get(&1), Some(&None));
    assert!(save
        .country_manager
        .database
        .get(&16777216)
        .and_then(|c| c.as_ref())
        .is_some());
    assert_eq!(save.building_manager.database.get(&2), Some(&None));
    assert_eq!(save.trade_route_manager.database.get(&2), Some(&None));
    assert_eq!(save.government_constructions.database.get(&1), Some(&None));
}

/// Uncompressed binary header (kind `01`) with no token map.
#[test]
fn binary_without_tokens_errors_clearly() {
    // SAV + version 01 + kind 01 (binary) + 8-byte random + 8-byte meta_len + LF
    let mut bytes = b"SAV0101deadbeef00000000\n".to_vec();
    bytes.extend_from_slice(&[0u8; 32]);

    let err = load_slice(&bytes, empty_tokens()).expect_err("binary needs tokens");
    assert!(
        matches!(err, LoadError::MissingTokens),
        "expected MissingTokens, got {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("token"),
        "error should mention tokens, got: {msg}"
    );
    assert!(
        !msg.to_lowercase().contains("missing field"),
        "must not be a serde mystery: {msg}"
    );
}

#[test]
#[ignore = "set VIC3_SAVE (and VIC3_TOKENS for binary) to run against a real save"]
fn live_save_from_env() {
    let save_path = std::env::var("VIC3_SAVE").expect("VIC3_SAVE must point at a .v3");
    let tokens = match std::env::var("VIC3_TOKENS") {
        Ok(path) => load_tokens_path(path).expect("VIC3_TOKENS must be a token map"),
        Err(_) => empty_tokens(),
    };
    let save = load_path(&save_path, &tokens).expect("live save should load");
    assert!(
        !save.meta_data.version.is_empty()
            || save.meta_data.game_date.is_some()
            || save.countries().next().is_some(),
        "live save produced an empty IR"
    );
}
