//! Paradox `database` maps store deleted slots as the identifier `none`
//! instead of an object. Serde would otherwise fail those entries.

use serde::de::{self, Deserialize, Deserializer, MapAccess, Unexpected, Visitor};
use std::{collections::HashMap, fmt, hash::Hash, marker::PhantomData};

/// Value that is either a deserialized `T` or Paradox `none`.
pub(crate) struct NoneOr<T>(pub Option<T>);

impl<'de, T> Deserialize<'de> for NoneOr<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct NoneOrVisitor<T> {
            marker: PhantomData<T>,
        }

        impl<'de, T> Visitor<'de> for NoneOrVisitor<T>
        where
            T: Deserialize<'de>,
        {
            type Value = NoneOr<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_fmt(format_args!(
                    "struct {} or none",
                    std::any::type_name::<T>()
                ))
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                match v {
                    "none" => Ok(NoneOr(None)),
                    _ => Err(E::invalid_value(Unexpected::Other(v), &self)),
                }
            }

            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                T::deserialize(de::value::MapAccessDeserializer::new(map)).map(|x| NoneOr(Some(x)))
            }
        }

        deserializer.deserialize_map(NoneOrVisitor {
            marker: PhantomData,
        })
    }
}

pub(crate) fn maybe_map<'de, D, K, V>(deser: D) -> Result<HashMap<K, Option<V>>, D::Error>
where
    D: Deserializer<'de>,
    K: Deserialize<'de> + Hash + Eq,
    V: Deserialize<'de>,
{
    struct MaybeVisitor<K, V> {
        marker: PhantomData<HashMap<K, Option<V>>>,
    }

    impl<'de, K, V> Visitor<'de> for MaybeVisitor<K, V>
    where
        K: Deserialize<'de> + Hash + Eq,
        V: Deserialize<'de>,
    {
        type Value = HashMap<K, Option<V>>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a database map of id = object or none")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut result = HashMap::with_capacity(map.size_hint().unwrap_or(0));
            while let Some((key, value)) = map.next_entry::<K, NoneOr<V>>()? {
                result.insert(key, value.0);
            }
            Ok(result)
        }
    }

    deser.deserialize_map(MaybeVisitor {
        marker: PhantomData,
    })
}

#[cfg(test)]
mod tests {
    use super::maybe_map;
    use serde::Deserialize;
    use std::collections::HashMap;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Country {
        definition: String,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Wrapper {
        #[serde(deserialize_with = "maybe_map")]
        database: HashMap<u32, Option<Country>>,
    }

    #[test]
    fn database_none_vs_object() {
        let mixed: Wrapper =
            jomini::text::de::from_utf8_slice(br#"database={ 1=none 2={ definition="GER" } }"#)
                .unwrap();
        assert_eq!(mixed.database.get(&1), Some(&None));
        assert_eq!(
            mixed.database.get(&2).and_then(|c| c.as_ref()),
            Some(&Country {
                definition: "GER".into()
            })
        );

        let only_none: Wrapper =
            jomini::text::de::from_utf8_slice(br#"database={ 101=none }"#).unwrap();
        assert_eq!(only_none.database.get(&101), Some(&None));

        jomini::text::de::from_utf8_slice::<Wrapper>(br#"database={ 101=None }"#)
            .expect_err("capitalized None is not the Paradox sentinel");
    }
}
