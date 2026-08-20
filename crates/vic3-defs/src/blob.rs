use serde::{Deserialize, Serialize};

use crate::{DefsError, GameDefs};

/// Postcard blob format version. Bump when [`GameDefs`] is not backward compatible.
///
/// [`decode_blob`] checks this **before** deserializing the payload so a stale
/// blob reports [`DefsError::BlobVersion`] instead of a confusing field error.
pub const BLOB_VERSION: u32 = 11;

#[derive(Serialize, Deserialize)]
struct DefsBlob {
    version: u32,
    defs: GameDefs,
}

/// Encode definitions into a compact, filesystem-free blob (postcard).
///
/// Intended for wasm: the CLI / desktop builds this from a game install or
/// fixture; the UI only calls [`decode_blob`]. Icons and flags are PNG bytes
/// already; DDS stays on disk at load time.
///
/// # Errors
///
/// Returns [`DefsError::BlobEncode`] on postcard failure.
pub fn encode_blob(defs: &GameDefs) -> Result<Vec<u8>, DefsError> {
    postcard::to_stdvec(&DefsBlob {
        version: BLOB_VERSION,
        defs: defs.clone(),
    })
    .map_err(DefsError::BlobEncode)
}

/// Decode a blob produced by [`encode_blob`].
///
/// The version is read on its own before the payload. A blob from an older
/// format does not deserialize as the current [`GameDefs`], so decoding both at
/// once would report whatever the payload happened to trip over — an invalid
/// UTF-8 string, say — instead of the version mismatch that actually explains it.
///
/// # Errors
///
/// - [`DefsError::BlobVersion`] — `found !=` [`BLOB_VERSION`]
/// - [`DefsError::BlobDecode`] — corrupt or truncated bytes
pub fn decode_blob(bytes: &[u8]) -> Result<GameDefs, DefsError> {
    let (version, payload) =
        postcard::take_from_bytes::<u32>(bytes).map_err(DefsError::BlobDecode)?;
    if version != BLOB_VERSION {
        return Err(DefsError::BlobVersion {
            found: version,
            expected: BLOB_VERSION,
        });
    }
    let mut defs: GameDefs = postcard::from_bytes(payload).map_err(DefsError::BlobDecode)?;
    crate::loc::polish_labels(&mut defs.labels);
    Ok(defs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BuyPackage, Good, GoodIdx, NeedEntry, NeedIdx, NeedsVec, PopNeed, ProductionMethod,
    };
    use std::collections::BTreeMap;

    fn sample_defs() -> GameDefs {
        let mut goods = BTreeMap::new();
        goods.insert(
            "grain".into(),
            Good {
                id: "grain".into(),
                base_price: 20.0,
                traded_quantity: 12.0,
                texture: None,
            },
        );
        goods.insert(
            "wood".into(),
            Good {
                id: "wood".into(),
                base_price: 20.0,
                traded_quantity: 10.0,
                texture: None,
            },
        );

        let mut production_methods = BTreeMap::new();
        production_methods.insert(
            "pm_simple_forestry".into(),
            ProductionMethod {
                id: "pm_simple_forestry".into(),
                inputs: vec![(GoodIdx::from_usize(0), 1.0)],
                outputs: vec![(GoodIdx::from_usize(1), 30.0)],
                ..ProductionMethod::default()
            },
        );

        let heat = NeedIdx::from_usize(0);
        let mut needs = NeedsVec::zeros(1);
        needs[heat] = 15.0;
        let mut buy_packages = BTreeMap::new();
        buy_packages.insert(
            1,
            BuyPackage {
                wealth: 1,
                political_strength: 0.03,
                needs,
            },
        );

        let mut obsessions = BTreeMap::new();
        obsessions.insert("french".into(), vec![GoodIdx::from_usize(0)]);

        let mut defs = GameDefs {
            price_range: 0.75,
            goods_order: vec!["grain".into(), "wood".into()],
            needs_order: vec!["popneed_heating".into()],
            goods,
            labels: BTreeMap::new(),
            icons: BTreeMap::new(),
            extra_icons: BTreeMap::new(),
            flags: BTreeMap::new(),
            flag_defs: BTreeMap::new(),
            production_methods,
            buildings: BTreeMap::new(),
            building_groups: BTreeMap::new(),
            pop_needs: vec![PopNeed {
                id: "popneed_heating".into(),
                default_good: Some(GoodIdx::from_usize(1)),
                entries: vec![NeedEntry {
                    good: GoodIdx::from_usize(1),
                    weight: 1.0,
                    min_supply_share: 0.0,
                    max_supply_share: 1.0,
                }],
            }],
            buy_packages,
            package_ladder: Vec::new(),
            obsessions,
            pop_types: BTreeMap::new(),
            production_method_groups: BTreeMap::new(),
        };
        defs.rebuild_package_ladder();
        defs
    }

    #[test]
    fn blob_round_trip() {
        let defs = sample_defs();
        let bytes = encode_blob(&defs).expect("encode");
        let decoded = decode_blob(&bytes).expect("decode");
        assert_eq!(decoded, defs);
    }

    #[test]
    fn rejects_previous_blob_version_clearly() {
        let bytes = postcard::to_stdvec(&DefsBlob {
            version: BLOB_VERSION - 1,
            defs: sample_defs(),
        })
        .expect("encode old version");
        let error = decode_blob(&bytes).expect_err("an older blob must require a rebuild");
        assert!(matches!(
            error,
            DefsError::BlobVersion {
                expected: BLOB_VERSION,
                ..
            }
        ));
    }

    /// The realistic stale blob: an older version whose payload no longer
    /// matches [`GameDefs`]. The version has to be reported, not the confusing
    /// decode failure the mismatched payload would raise.
    #[test]
    fn reports_the_version_even_when_the_payload_shape_changed() {
        let bytes = postcard::to_stdvec(&(BLOB_VERSION - 1, "a payload from an older format"))
            .expect("encode old blob");
        let error = decode_blob(&bytes).expect_err("an older blob must require a rebuild");
        assert!(
            matches!(
                error,
                DefsError::BlobVersion {
                    found,
                    expected: BLOB_VERSION,
                } if found == BLOB_VERSION - 1
            ),
            "expected a version mismatch, got {error}"
        );
    }

    #[test]
    fn rejects_bytes_that_are_not_a_blob() {
        let error = decode_blob(&[]).expect_err("empty input is not a blob");
        assert!(matches!(error, DefsError::BlobDecode(_)), "got {error}");
    }
}
