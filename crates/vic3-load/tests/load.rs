//! Integration tests for `vic3-load`.

use std::path::PathBuf;
use vic3_load::{
    empty_tokens, load_path, load_path_world, load_slice, load_tokens_path, LoadError,
    WorldSnapshot,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn toy_economy_fixture_loads() {
    let save = load_path(fixture("toy_economy.txt"), empty_tokens()).expect("toy economy fixture");

    assert_eq!(save.meta_data.version, "1.9.0");
    assert_eq!(save.meta_data.name.as_deref(), Some("ToyEconomy"));

    let toy = save.country_by_tag("TOY").expect("TOY in fixture");
    assert_eq!(toy.definition, "TOY");
    assert_eq!(toy.market, Some(1));
    assert_eq!(toy.states, vec![1, 2, 3]);
    assert_eq!(save.states.iter_present().count(), 3);
    assert_eq!(
        save.states
            .database
            .get(&1)
            .and_then(|s| s.as_ref())
            .and_then(|s| s.region.as_deref()),
        Some("STATE_TOY_A")
    );
    assert_eq!(
        save.states
            .database
            .get(&2)
            .and_then(|s| s.as_ref())
            .and_then(|s| s.region.as_deref()),
        Some("STATE_TOY_B")
    );
    assert_eq!(
        save.states
            .database
            .get(&3)
            .and_then(|s| s.as_ref())
            .and_then(|s| s.region.as_deref()),
        Some("STATE_TOY_C")
    );

    let buildings: Vec<_> = save.building_manager.iter_present().collect();
    assert_eq!(buildings.len(), 4);
    assert!(buildings
        .iter()
        .any(|(_, b)| b.building == "building_wheat_farm"
            && b.active_production_methods() == ["pm_toy_wheat"]));
    assert!(buildings
        .iter()
        .any(|(_, b)| b.building == "building_flour_mill"
            && b.active_production_methods() == ["pm_toy_mill"]));
    assert!(buildings
        .iter()
        .any(|(_, b)| b.building == "building_bakery"
            && b.active_production_methods() == ["pm_toy_bakery"]));
    let trade = buildings
        .iter()
        .find(|(_, b)| b.building == "building_trade_center")
        .map(|(_, b)| b)
        .expect("trade center");
    assert_eq!(trade.state, Some(3));
    assert_eq!(trade.active_production_methods(), ["pm_toy_trade"]);

    let farm = buildings
        .iter()
        .find(|(_, b)| b.building == "building_wheat_farm")
        .map(|(_, b)| b)
        .expect("wheat farm");
    assert_eq!(farm.level, 3);
    assert_eq!(farm.output_goods.goods.get("0"), Some(&90.0));

    let mill = buildings
        .iter()
        .find(|(_, b)| b.building == "building_flour_mill")
        .map(|(_, b)| b)
        .expect("flour mill");
    assert_eq!(mill.input_goods.goods.get("0"), Some(&40.0));
    assert_eq!(mill.output_goods.goods.get("1"), Some(&30.0));

    let bakery = buildings
        .iter()
        .find(|(_, b)| b.building == "building_bakery")
        .map(|(_, b)| b)
        .expect("bakery");
    assert_eq!(bakery.input_goods.goods.get("1"), Some(&30.0));
    assert_eq!(bakery.output_goods.goods.get("2"), Some(&40.0));

    assert_eq!(save.pops.iter_present().count(), 4);
    assert_eq!(save.market_manager.iter_present().count(), 1);
    assert_eq!(
        save.market_manager
            .iter_present()
            .next()
            .and_then(|(_, market)| market.owner),
        Some(16777216)
    );
    assert_eq!(WorldSnapshot::previous_played(&save).len(), 1);
    assert_eq!(
        WorldSnapshot::previous_played(&save)[0].idtype,
        Some(16777216)
    );
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
    assert_eq!(ger.budget.principal, Some(0.0));
    assert_eq!(ger.budget.weekly_income, vec![0.0, 100.0]);
    assert_eq!(ger.budget.credit_headroom(), Some(500.0));
    assert!(ger.budget.is_solvent());
    assert_eq!(ger.market, Some(1));

    let state = save.states.database.get(&1).and_then(|s| s.as_ref());
    assert_eq!(
        state.and_then(|s| s.region.as_deref()),
        Some("STATE_BRANDENBURG")
    );
    assert_eq!(state.and_then(|s| s.arable_land), Some(45.0));
    assert_eq!(state.and_then(|s| s.infrastructure), Some(32.5));
    assert_eq!(state.and_then(|s| s.infrastructure_usage), Some(21.0));
    assert_eq!(
        state.and_then(|s| s.trade.goods.get("0")).copied(),
        Some(5.0)
    );
    assert_eq!(
        state.and_then(|s| s.trade.goods.get("1")).copied(),
        Some(-3.0)
    );

    let farm = save
        .building_manager
        .iter_present()
        .find(|(_, b)| b.building == "building_rye_farm")
        .map(|(_, b)| b)
        .expect("rye farm");
    assert_eq!(farm.level, 2);
    assert_eq!(farm.staffing, 1.5);
    assert_eq!(farm.state, Some(1));
    assert_eq!(farm.active_production_methods(), ["pm_simple_forestry"]);
    assert_eq!(farm.input_goods.goods.get("0"), Some(&5.0));
    assert_eq!(farm.output_goods.goods.get("1"), Some(&40.0));

    let pop = save
        .pops
        .iter_present()
        .next()
        .map(|(_, p)| p)
        .expect("pop");
    assert_eq!(pop.profession.as_deref(), Some("farmers"));
    assert_eq!(pop.workforce, Some(6000.0));
    assert_eq!(pop.dependents, Some(4000.0));
    assert_eq!(pop.demand_size(), Some(10000.0));
    assert_eq!(pop.wealth, Some(8));
    assert_eq!(pop.culture.as_deref(), Some("0"));
    assert_eq!(
        save.culture_id(pop.culture.as_deref()).as_deref(),
        Some("north_german")
    );
    assert_eq!(pop.state, Some(1));
    assert_eq!(pop.workplace, Some(1));
    assert_eq!(pop.literate, Some(1200.0));
    assert_eq!(pop.qualifications.values.get("0"), Some(&1.5));
    assert_eq!(pop.qualifications.values.get("6"), Some(&2.0));
    assert_eq!(
        state
            .and_then(|s| s.pop_statistics.population_by_profession.values.get("7"))
            .copied(),
        Some(10000.0)
    );
    assert_eq!(
        state.and_then(|s| s.employable().values.get("0")).copied(),
        Some(2.0)
    );
    assert_eq!(
        state
            .and_then(|s| s.workforce_by_profession().values.get("7"))
            .copied(),
        Some(6000.0)
    );

    assert_eq!(save.market_manager.iter_present().count(), 1);
    assert_eq!(
        save.market_manager
            .iter_present()
            .next()
            .and_then(|(_, market)| market.owner),
        Some(16777216)
    );
    assert_eq!(save.active_laws(16777216), ["law_autocracy"]);
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
        .map(|(_, o)| o);
    assert!(
        order.is_none(),
        "shared fixture leaves construction queue empty"
    );
    assert!(ger.techs.iter().any(|tech| tech == "railways"));
    assert!(ger.techs.iter().any(|tech| tech == "urban_planning"));
    assert!(ger.currently_researching.is_none());
    assert!(save.queued_building_for(16777216).is_none());
}

#[test]
fn construction_queue_and_research_head_parse() {
    let save = load_slice(
        br#"SAV01000000000000000000
country_manager={
	database={
		16777216={ definition="GER" states={ 1 } }
	}
}
states={
	database={
		1={ country=16777216 }
	}
}
building_constructions={
	database={
		1={
			building="building_construction_sector"
			state=1
			remaining=20
		}
	}
}
technology={
	database={
		1={
			country=16777216
			acquired_technologies={ value={ railways } }
			research_technology=atmospheric_engine
		}
	}
}
"#,
        empty_tokens(),
    )
    .expect("queue fixture");
    let ger = save.country_by_tag("GER").expect("GER");
    assert!(ger.techs.iter().any(|tech| tech == "railways"));
    assert_eq!(
        ger.currently_researching.as_deref(),
        Some("atmospheric_engine")
    );
    assert_eq!(
        save.queued_building_for(16777216).as_deref(),
        Some("building_construction_sector")
    );
}

#[test]
fn plaintext_world_save_skips_market_ir() {
    let save = load_path_world(fixture("plaintext.txt"), empty_tokens()).expect("world save");
    assert_eq!(save.pops.iter_present().count(), 1);
    assert_eq!(save.active_laws(16777216), ["law_autocracy"]);
    assert_eq!(WorldSnapshot::previous_played(&save).len(), 1);
    let ger = save
        .country_manager
        .database
        .get(&16777216)
        .and_then(Option::as_ref)
        .expect("GER");
    assert!(ger.techs.iter().any(|tech| tech == "urban_planning"));
    assert!(ger.currently_researching.is_none());
    assert_eq!(
        save.building_constructions.iter_present().count(),
        0,
        "shared fixture leaves construction queue empty"
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
            "production_methods={ \"pm_simple_forestry\" }",
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
fn legacy_population_and_level_aliases_still_load() {
    let text = std::fs::read_to_string(fixture("plaintext.txt"))
        .unwrap()
        .replace("workforce=6000", "size_wa=6000")
        .replace("dependents=4000", "size_dn=4000")
        .replace("levels=2", "level=2");
    let save = load_slice(text.as_bytes(), empty_tokens()).expect("legacy aliases");
    let pop = save.pops.iter_present().next().unwrap().1;
    let building = save.building_manager.iter_present().next().unwrap().1;
    assert_eq!(pop.demand_size(), Some(10_000.0));
    assert_eq!(building.level, 2);
}

#[test]
fn literate_alias_still_loads() {
    let text = std::fs::read_to_string(fixture("plaintext.txt"))
        .unwrap()
        .replace("num_literate=1200", "literate=1200");
    let save = load_slice(text.as_bytes(), empty_tokens()).expect("literate alias");
    let pop = save.pops.iter_present().next().unwrap().1;
    assert_eq!(pop.literate, Some(1200.0));
}

#[test]
fn malformed_building_goods_are_not_silently_discarded() {
    let text = std::fs::read_to_string(fixture("plaintext.txt"))
        .unwrap()
        .replace("0={ value=5 }", "0={ value=\"not-a-number\" }");
    assert!(
        load_slice(text.as_bytes(), empty_tokens()).is_err(),
        "invalid saved IO must fail parsing"
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
    let (country_id, country) = save
        .previous_played
        .iter()
        .find_map(|player| {
            let id = player.idtype?;
            save.country_manager
                .database
                .get(&id)
                .and_then(Option::as_ref)
                .map(|country| (id, country))
        })
        .expect("previous_played.idtype should resolve to a country");
    assert_eq!(country.definition, "PRU");
    assert!(country.market.is_some());
    assert!(!save.active_laws(country_id).is_empty());
    assert!(
        save.building_manager.iter_present().any(|(_, building)| {
            !building.input_goods.goods.is_empty() || !building.output_goods.goods.is_empty()
        }),
        "real save should contain saved building goods orders"
    );
    let (_, pop) = save
        .pops
        .iter_present()
        .find(|(_, pop)| pop.culture.as_deref() == Some("0"))
        .expect("real save should have a culture index 0 pop");
    assert_eq!(
        save.culture_id(pop.culture.as_deref()).as_deref(),
        Some("north_german"),
        "cultures.database[0].type should be north_german in vanilla"
    );
}
