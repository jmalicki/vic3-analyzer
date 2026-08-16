use serde::{Deserialize, Serialize};

use crate::{DefsError, GameDefs};

/// Postcard blob format version. Bump when [`GameDefs`] is not backward compatible.
pub const BLOB_VERSION: u32 = 2;

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
pub fn decode_blob(bytes: &[u8]) -> Result<GameDefs, DefsError> {
    let blob: DefsBlob = postcard::from_bytes(bytes).map_err(DefsError::BlobDecode)?;
    if blob.version != BLOB_VERSION {
        return Err(DefsError::BlobVersion {
            found: blob.version,
            expected: BLOB_VERSION,
        });
    }
    Ok(blob.defs)
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
            goods,
            labels: BTreeMap::new(),
            production_methods,
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
            version: 1,
            defs: sample_defs(),
        })
        .expect("encode old version");
        let error = decode_blob(&bytes).expect_err("v1 must require rebuild");
        assert!(matches!(
            error,
            DefsError::BlobVersion {
                found: 1,
                expected: BLOB_VERSION
            }
        ));
    }
}
