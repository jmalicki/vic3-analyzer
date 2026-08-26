//! Smoke-load the self-contained toy economy fixture tree.

use std::path::PathBuf;

use vic3_defs::load_from_path;

fn toy_economy_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/toy_economy")
}

#[test]
fn toy_economy_loads_goods_buildings_and_mill_pms() {
    let defs = load_from_path(toy_economy_root()).expect("toy economy fixture should parse");

    assert_eq!(defs.base_price("wheat"), Some(20.0));
    assert_eq!(defs.base_price("flour"), Some(30.0));
    assert_eq!(defs.base_price("bread"), Some(40.0));
    assert!(defs.goods["wheat"].traded_quantity > 0.0);
    assert!(defs.goods["bread"].traded_quantity > 0.0);

    for building in [
        "building_wheat_farm",
        "building_flour_mill",
        "building_bakery",
        "building_trade_center",
    ] {
        assert!(
            defs.building_types.contains_key(building),
            "missing building type {building}"
        );
    }

    let mill_group = &defs.building_types["building_flour_mill"].production_method_groups;
    assert_eq!(mill_group, &["pmg_base_building_flour_mill".to_string()]);
    assert_eq!(
        defs.building_types["building_flour_mill"].required_construction,
        Some(400.0)
    );
    assert_eq!(
        defs.building_types["building_wheat_farm"].required_construction,
        Some(200.0)
    );
    assert_eq!(
        defs.production_method_groups["pmg_base_building_flour_mill"],
        ["pm_toy_mill", "pm_toy_mill_efficient"]
    );
    assert!(defs.production_methods.contains_key("pm_toy_mill"));
    assert!(defs
        .production_methods
        .contains_key("pm_toy_mill_efficient"));
}
