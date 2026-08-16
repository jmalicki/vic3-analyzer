//! Load the Clausewitz fixture tree and check public defs.

use std::path::PathBuf;

use vic3_defs::{decode_blob, encode_blob, load_from_files, load_from_path, DEFAULT_PRICE_RANGE};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load_fixture() -> vic3_defs::GameDefs {
    load_from_path(fixture_root()).expect("fixture tree should parse")
}

#[test]
fn fixture_goods_have_known_base_prices() {
    let defs = load_fixture();
    assert_eq!(defs.base_price("grain"), Some(20.0));
    assert_eq!(defs.base_price("wood"), Some(20.0));
    assert_eq!(defs.base_price("coal"), Some(30.0));
    assert_eq!(defs.goods.len(), 3);
    assert_eq!(defs.labels.get("grain").map(String::as_str), Some("Grain"));
    assert_eq!(
        defs.goods["grain"].texture.as_deref(),
        Some("gfx/interface/icons/goods_icons/grain.dds")
    );
}

#[test]
fn fixture_goods_icons_are_decoded_to_png() {
    let defs = load_fixture();
    let grain = defs.icons.get("grain").expect("grain has a DDS icon");
    assert_eq!(&grain[1..4], b"PNG");
    // Only goods whose texture resolves get an icon.
    assert_eq!(defs.icons.len(), 1);
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
    let need = defs.pop_needs.get("popneed_heating").expect("heating need");
    assert_eq!(need.default_good.as_deref(), Some("wood"));
    assert_eq!(need.entries.len(), 2);
    assert_eq!(need.entries[0].good, "wood");
    assert!((need.entries[0].max_supply_share - 0.5).abs() < f64::EPSILON);
    assert_eq!(need.entries[1].good, "coal");
    assert!((need.entries[1].min_supply_share - 0.1).abs() < f64::EPSILON);

    assert_eq!(defs.buy_packages.len(), 2);
    assert_eq!(
        defs.buy_packages[&1].needs.get("popneed_heating").copied(),
        Some(15.0)
    );
    assert_eq!(
        defs.buy_packages[&2].needs.get("popneed_heating").copied(),
        Some(17.0)
    );
}

#[test]
fn fixture_production_methods_have_goods_io() {
    let defs = load_fixture();
    let forestry = defs
        .production_methods
        .get("pm_simple_forestry")
        .expect("forestry PM");
    assert_eq!(forestry.outputs.get("wood").copied(), Some(30.0));
    assert_eq!(forestry.inputs.get("tools").copied(), Some(1.0));
    let mining = defs
        .production_methods
        .get("pm_simple_mining")
        .expect("mining PM");
    assert_eq!(mining.outputs.get("coal").copied(), Some(25.0));
}

#[test]
fn fixture_obsessions_empty() {
    let defs = load_fixture();
    assert!(defs.obsessions.is_empty());
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
                collect(root, &path, out);
            } else if path.extension().is_some_and(|extension| {
                extension == "txt" || extension == "yml" || extension == "dds"
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
