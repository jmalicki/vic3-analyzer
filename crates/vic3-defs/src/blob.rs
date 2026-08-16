use serde::{Deserialize, Serialize};

use crate::{DefsError, GameDefs};

/// Postcard blob format version. Bump when [`GameDefs`] is not backward compatible.
pub const BLOB_VERSION: u32 = 6;

#[derive(Serialize, Deserialize)]
struct DefsBlob {
    version: u32,
    defs: GameDefs,
}

/// Encode definitions into a compact, filesystem-free blob (postcard).
///
/// Intended for wasm: the CLI builds this from a game install; the UI only
/// calls [`decode_blob`].
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
pub fn decode_blob(bytes: &[u8]) -> Result<GameDefs, DefsError> {
    let (version, payload) =
        postcard::take_from_bytes::<u32>(bytes).map_err(DefsError::BlobDecode)?;
    if version != BLOB_VERSION {
        return Err(DefsError::BlobVersion {
            found: version,
            expected: BLOB_VERSION,
        });
    }
    postcard::from_bytes(payload).map_err(DefsError::BlobDecode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BuyPackage, Good, NeedEntry, PopNeed, ProductionMethod};
    use std::collections::BTreeMap;

    fn sample_defs() -> GameDefs {
        let mut goods = BTreeMap::new();
        goods.insert(
            "grain".into(),
            Good {
                id: "grain".into(),
                base_price: 20.0,
                texture: None,
            },
        );
        goods.insert(
            "wood".into(),
            Good {
                id: "wood".into(),
                base_price: 20.0,
                texture: None,
            },
        );

        let mut inputs = BTreeMap::new();
        inputs.insert("tools".into(), 1.0);
        let mut outputs = BTreeMap::new();
        outputs.insert("wood".into(), 30.0);
        let mut production_methods = BTreeMap::new();
        production_methods.insert(
            "pm_simple_forestry".into(),
            ProductionMethod {
                id: "pm_simple_forestry".into(),
                inputs,
                outputs,
            },
        );

        let mut pop_needs = BTreeMap::new();
        pop_needs.insert(
            "popneed_heating".into(),
            PopNeed {
                id: "popneed_heating".into(),
                default_good: Some("wood".into()),
                entries: vec![NeedEntry {
                    good: "wood".into(),
                    weight: 1.0,
                    min_supply_share: 0.0,
                    max_supply_share: 1.0,
                }],
            },
        );

        let mut needs = BTreeMap::new();
        needs.insert("popneed_heating".into(), 15.0);
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
        obsessions.insert("french".into(), vec!["wine".into()]);

        GameDefs {
            price_range: 0.75,
            goods_order: vec!["grain".into(), "wood".into()],
            goods,
            labels: BTreeMap::new(),
            icons: BTreeMap::new(),
            flags: BTreeMap::new(),
            flag_defs: BTreeMap::new(),
            production_methods,
            buildings: BTreeMap::new(),
            building_groups: BTreeMap::new(),
            pop_needs,
            buy_packages,
            obsessions,
        }
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
