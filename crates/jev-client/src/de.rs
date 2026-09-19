//! Deserialisation helpers shared by the request types.

use std::fmt;
use std::marker::PhantomData;

use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::{Deserializer, Error, MapAccess, Visitor};

/// Deserialises a field that was present, keeping an explicit `null` distinct from an absent key.
///
/// Used with `#[serde(default)]` on an `Option<T>` whose `T` can itself represent `null`: an
/// absent key stays `None`, while `"key": null` becomes `Some(null)`. The request is then sent
/// exactly as the caller wrote it.
pub(crate) fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// Deserialises a map in document order and rejects a repeated key.
///
/// Parsers normally keep the last of two identical keys, which would silently drop a question or
/// a choice option the caller wrote. Failing loudly is cheaper than an answer nobody asked for.
pub(crate) fn unique_map<'de, D, V>(deserializer: D) -> Result<IndexMap<String, V>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    struct UniqueMap<V>(PhantomData<V>);

    impl<'de, V: Deserialize<'de>> Visitor<'de> for UniqueMap<V> {
        type Value = IndexMap<String, V>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a map with unique keys")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
            let mut map = IndexMap::with_capacity(access.size_hint().unwrap_or(0).min(256));
            while let Some(key) = access.next_key::<String>()? {
                if map.contains_key(&key) {
                    return Err(A::Error::custom(format_args!("duplicate key `{key}`")));
                }
                let value = access.next_value()?;
                map.insert(key, value);
            }
            Ok(map)
        }
    }

    deserializer.deserialize_map(UniqueMap(PhantomData))
}
