//! Load the Clausewitz fixture tree and check public defs.

use std::path::PathBuf;

use vic3_defs::{
    decode_blob, encode_blob, load_from_files, load_from_path, GoodIdx, DEFAULT_PRICE_RANGE,
    DEFAULT_TRADED_QUANTITY,
};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load_fixture() -> vic3_defs::GameDefs {
    load_from_path(fixture_root()).expect("fixture tree should parse")
}

#[test]
fn pop_type_labels_resolve_icon_markup() {
    let defs = load_from_files([
        (
            "game/common/goods/00_goods.txt".to_string(),
            b"grain = { cost = 20 }\n".to_vec(),
        ),
        (
            "game/localization/english/pop_types_l_english.yml".to_string(),
            br#"l_english:
 academics:0 "@academics! $academics_no_icon$"
 academics_no_icon:0 "Academics"
 farmers:0 "@farmers! $farmers_no_icon$"
 farmers_no_icon:0 "Farmers"
"#
            .to_vec(),
        ),
    ])
    .expect("pop type loc");
    assert_eq!(
        defs.labels.get("academics").map(String::as_str),
        Some("Academics")
    );
    assert_eq!(
        defs.labels.get("farmers").map(String::as_str),
        Some("Farmers")
    );
}

#[test]
fn production_method_icons_alias_script_id_onto_texture_stem() {
    let dds = std::fs::read(fixture_root().join("gfx/interface/icons/goods_icons/grain.dds"))
        .expect("fixture dds");
    let defs = load_from_files([
        (
            "game/common/goods/00_goods.txt".to_string(),
            b"grain = { cost = 20 }\n".to_vec(),
        ),
        (
            "game/common/production_methods/00_production_methods.txt".to_string(),
            br#"pm_bakery = {
	texture = "gfx/interface/icons/production_method_icons/bakeries.dds"
	building_modifiers = { workforce_scaled = { goods_output_grain_add = 1 } }
}
"#
            .to_vec(),
        ),
        (
            "game/gfx/interface/icons/production_method_icons/bakeries.dds".to_string(),
            dds,
        ),
    ])
    .expect("pm texture alias");
    let by_stem = defs
        .extra_icons
        .get("pm:bakeries")
        .expect("stem key from filename");
    let by_id = defs
        .extra_icons
        .get("pm:pm_bakery")
        .expect("script id aliased onto texture stem");
    assert_eq!(by_stem, by_id);
    assert_eq!(&by_id[1..4], b"PNG");
}

#[test]
fn fixture_goods_have_known_base_prices() {
    let defs = load_fixture();
    assert_eq!(defs.base_price("grain"), Some(20.0));
    assert_eq!(defs.base_price("wood"), Some(20.0));
    assert_eq!(defs.base_price("coal"), Some(30.0));
    assert_eq!(defs.goods["grain"].traded_quantity, 12.0);
    assert_eq!(defs.goods["wood"].traded_quantity, DEFAULT_TRADED_QUANTITY);
    assert_eq!(defs.goods["coal"].traded_quantity, 6.0);
    assert_eq!(defs.goods.len(), 3);
    assert_eq!(defs.goods_order, ["grain", "wood", "coal"]);
    assert_eq!(defs.good_by_index(GoodIdx::from_usize(1)), Some("wood"));
    assert_eq!(defs.labels.get("grain").map(String::as_str), Some("Grain"));
    assert_eq!(
        defs.goods["grain"].texture.as_deref(),
        Some("gfx/interface/icons/goods_icons/grain.dds")
    );
}

#[test]
fn in_memory_goods_preserve_source_order() {
    let mut source = String::new();
    for index in 0..18 {
        source.push_str(&format!("good_{index} = {{ cost = 10 }}\n"));
    }
    source.push_str("merchant_marine = { cost = 15 }\n");
    let defs = load_from_files([(
        "game/common/goods/00_goods.txt".to_string(),
        source.into_bytes(),
    )])
    .expect("ordered goods");
    assert_eq!(
        defs.good_by_index(GoodIdx::from_usize(18)),
        Some("merchant_marine")
    );
}

#[test]
fn fixture_goods_icons_are_decoded_to_png() {
    let defs = load_fixture();
    let grain = defs.icons.get("grain").expect("grain has a DDS icon");
    assert_eq!(&grain[1..4], b"PNG");
    // Only goods whose texture resolves get an icon.
    assert_eq!(defs.icons.len(), 1);
    let farm = defs
        .extra_icons
        .get("building:building_rye_farm")
        .expect("building icon is keyed by stem");
    assert_eq!(&farm[1..4], b"PNG");
}

#[test]
fn fixture_price_range_from_neconomy() {
    let defs = load_fixture();
    assert!((defs.price_range - DEFAULT_PRICE_RANGE).abs() < f64::EPSILON);
    assert!((defs.price_range - 0.75).abs() < f64::EPSILON);
}

#[test]
fn fixture_need_and_wealth_packages() {
    let defs = load_fixture();
    let need = defs.pop_need("popneed_heating").expect("heating need");
    assert_eq!(need.default_good, defs.index_of("wood"));
    assert_eq!(need.entries.len(), 2);
    assert_eq!(need.entries[0].good, defs.index_of("wood").unwrap());
    assert!((need.entries[0].max_supply_share - 0.5).abs() < f64::EPSILON);
    assert_eq!(need.entries[1].good, defs.index_of("coal").unwrap());
    assert!((need.entries[1].min_supply_share - 0.1).abs() < f64::EPSILON);

    assert_eq!(defs.needs_order, ["popneed_heating"]);
    assert_eq!(defs.buy_packages.len(), 2);
    let heat = defs.need_index_of("popneed_heating").unwrap();
    assert_eq!(defs.buy_packages[&1].needs[heat], 15.0);
    assert_eq!(defs.buy_packages[&2].needs[heat], 17.0);
    assert_eq!(defs.package_ladder.len(), 99);
    assert_eq!(defs.package_ladder[0][heat], 15.0);
    assert_eq!(defs.package_ladder[1][heat], 17.0);
}

#[test]
fn fixture_production_methods_have_goods_io() {
    let defs = load_fixture();
    let qty = |rows: &[(GoodIdx, f64)], id: &str| {
        let idx = defs.index_of(id)?;
        rows.iter()
            .find_map(|(good, qty)| (*good == idx).then_some(*qty))
    };
    let forestry = defs
        .production_methods
        .get("pm_simple_forestry")
        .expect("forestry PM");
    assert_eq!(qty(&forestry.outputs, "wood"), Some(30.0));
    // The fixture's tools input has no matching good definition, so indexed
    // runtime data drops it rather than carrying an unresolvable string.
    assert_eq!(qty(&forestry.inputs, "tools"), None);
    let mining = defs
        .production_methods
        .get("pm_simple_mining")
        .expect("mining PM");
    assert_eq!(qty(&mining.outputs, "coal"), Some(25.0));
}

#[test]
fn fixture_pop_types_record_qualification_sources() {
    let defs = load_fixture();
    let aristocrats = defs.pop_types.get("aristocrats").expect("aristocrats");
    assert!(!aristocrats.can_always_hire);
    assert!(aristocrats.qualifications.wealth);
    assert!(aristocrats.qualifications.literacy);
    assert_eq!(aristocrats.qualifications.wealth_floor, Some(10.0));
    let capitalists = defs.pop_types.get("capitalists").expect("capitalists");
    assert!(capitalists.qualifications.wealth);
    assert_eq!(capitalists.qualifications.wealth_floor, Some(20.0));
    assert_eq!(
        aristocrats.qualifications.source_multipliers.get("farmers"),
        Some(&2.0)
    );
    assert_eq!(
        aristocrats
            .qualifications
            .source_multipliers
            .get("bureaucrats"),
        Some(&5.0)
    );
    assert!(defs.pop_types["laborers"].can_always_hire);
    let machinists = &defs.pop_types["machinists"];
    assert!(machinists.qualifications.literacy);
    assert_eq!(
        machinists.qualifications.source_multipliers.get("laborers"),
        Some(&3.0)
    );
    let farm_pm = defs
        .production_methods
        .get("pm_simple_farming")
        .expect("farming PM");
    assert!(farm_pm
        .employment
        .iter()
        .any(|(prof, qty)| prof == "farmers" && (*qty - 4000.0).abs() < f64::EPSILON));
    let uni = defs
        .production_methods
        .get("pm_scholastic_education")
        .expect("university PM");
    assert!(uni.education_access);
    assert!(uni.qualifications_boost);
    assert_eq!(
        defs.buildings["building_rye_farm"].production_method_groups,
        ["pmg_base_building_rye_farm"]
    );
    assert_eq!(
        defs.buildings["building_rye_farm"].required_construction,
        Some(200.0)
    );
    assert_eq!(
        defs.production_method_groups["pmg_base_building_rye_farm"],
        ["pm_simple_farming"]
    );
}

#[test]
fn fixture_buildings_have_state_panel_metadata() {
    let defs = load_fixture();
    let farm = &defs.buildings["building_rye_farm"];
    assert_eq!(farm.group.as_deref(), Some("bg_agriculture"));
    assert_eq!(farm.city_type.as_deref(), Some("farm"));
    let agriculture = &defs.building_groups["bg_agriculture"];
    assert_eq!(agriculture.category.as_deref(), Some("rural"));
    assert_eq!(agriculture.land_usage.as_deref(), Some("rural"));
    assert!(agriculture.always_possible);
    assert_eq!(
        agriculture.default_building.as_deref(),
        Some("building_rye_farm")
    );
    assert_eq!(
        defs.labels.get("building_rye_farm").map(String::as_str),
        Some("Rye Farms")
    );
}

#[test]
fn fixture_obsessions_empty() {
    let defs = load_fixture();
    assert!(defs.obsessions.is_empty());
}

#[test]
fn fixture_coa_flag_and_country_label() {
    let defs = load_fixture();
    assert!(defs.flags.contains_key("TST"), "solid CoA should render");
    assert_eq!(defs.labels.get("TST").map(String::as_str), Some("Testopia"));
    let selected = vic3_defs::select_flag_coa(&defs.flag_defs, &defs.flags, "TST", &[]);
    assert_eq!(selected.as_deref(), Some("TST"));
}

#[test]
fn blob_round_trip_from_fixture() {
    let defs = load_fixture();
    let bytes = encode_blob(&defs).expect("encode blob");
    let decoded = decode_blob(&bytes).expect("decode blob");
    assert_eq!(decoded, defs);
}

#[test]
fn load_accepts_game_subdirectory_layout() {
    // Same fixture is already a data root (`common/` at top). A missing `game/`
    // wrapper must still resolve via `common/goods`.
    let defs = load_from_path(fixture_root()).unwrap();
    assert!(defs.goods.contains_key("grain"));
}

#[test]
fn in_memory_files_match_filesystem_loader() {
    fn collect(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                // Sibling self-contained trees (e.g. toy_economy/) must not mix in.
                if path.file_name().is_some_and(|name| name == "toy_economy") {
                    continue;
                }
                collect(root, &path, out);
            } else if path.extension().is_some_and(|extension| {
                extension == "txt" || extension == "yml" || extension == "dds" || extension == "tga"
            }) {
                out.push((
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                    std::fs::read(path).unwrap(),
                ));
            }
        }
    }

    let root = fixture_root();
    let mut files = Vec::new();
    collect(&root, &root, &mut files);
    let memory = load_from_files(files).expect("in-memory fixture tree");
    assert_eq!(memory, load_fixture());
}

#[test]
fn one_file_at_a_time_matches_a_single_batch() {
    fn collect(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                // Sibling self-contained trees (e.g. toy_economy/) must not mix in.
                if path.file_name().is_some_and(|name| name == "toy_economy") {
                    continue;
                }
                collect(root, &path, out);
            } else if path.extension().is_some_and(|extension| {
                extension == "txt" || extension == "yml" || extension == "dds" || extension == "tga"
            }) {
                out.push((
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                    std::fs::read(path).unwrap(),
                ));
            }
        }
    }

    let root = fixture_root();
    let mut files = Vec::new();
    collect(&root, &root, &mut files);
    // The browser submits batches as it reads them, so arrival order must not
    // change the result the way a single sorted pass would.
    files.reverse();
    let mut builder = vic3_defs::DefsBuilder::new();
    for file in files.clone() {
        builder.add_files([file]).expect("streamed file");
    }
    assert_eq!(builder.finish().expect("streamed fixture tree"), {
        files.reverse();
        load_from_files(files).expect("in-memory fixture tree")
    });
}

#[test]
fn builder_parses_text_when_the_file_arrives() {
    let mut builder = vic3_defs::DefsBuilder::new();
    let error = builder
        .add_files([(
            "game/common/goods/00_goods.txt".to_string(),
            b"grain = { ".to_vec(),
        )])
        .expect_err("malformed goods should fail on add, not finish");
    assert!(
        error.to_string().contains("parse"),
        "unexpected error: {error}"
    );
}

#[test]
#[ignore = "requires VIC3_GAME pointing at a Victoria 3 install or game directory"]
fn live_game_renders_representative_flags_without_magenta() {
    let root = std::env::var_os("VIC3_GAME").expect("set VIC3_GAME");
    let defs = load_from_path(root).expect("real game definitions should load");
    for id in ["PRU", "GBR", "FRA"] {
        let flag = defs
            .flags
            .get(id)
            .unwrap_or_else(|| panic!("{id} should render"));
        let decoder = png::Decoder::new(std::io::Cursor::new(flag));
        let mut reader = decoder.read_info().expect("valid PNG");
        let mut pixels = vec![0; reader.output_buffer_size().expect("buffer size")];
        let info = reader.next_frame(&mut pixels).expect("PNG pixels");
        let pixels = &pixels[..info.buffer_size()];
        assert!(
            !pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel == &[200, 0, 200, 255]),
            "{id} must not contain the old unknown-color placeholder"
        );
    }
}
