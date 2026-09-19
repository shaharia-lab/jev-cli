//! A request as it was written: the JSON value, plus the duplicate keys a plain parse would hide.

use std::fmt;

use serde::de::{DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

use crate::request::Request;

/// A request document, parsed for validation rather than for sending.
///
/// Validation works on the raw document, not on a typed [`Request`], for two reasons. A typed parse
/// stops at the first problem and reports it without saying which question it is in; and any
/// ordinary parse silently keeps the last of two identical keys, so a repeated question id could
/// never be reported. A `Document` keeps the whole value whatever its shape, and remembers every
/// duplicate key it saw.
///
/// It implements [`Deserialize`] and does its work in a single pass, asking only that the format
/// be self-describing. [`Document::from_json_str`] covers JSON; a caller reading another such
/// format, YAML for instance, deserialises a `Document` with that format's own deserializer.
///
/// ```
/// use jev_client::validate::Document;
///
/// let document = Document::from_json_str(r#"{"state": "s", "state": "again"}"#).unwrap();
/// assert_eq!(document.value()["state"], "again");
/// assert_eq!(document.duplicate_keys().len(), 1);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    value: Value,
    duplicates: Vec<DuplicateKey>,
}

/// A key that appeared more than once in one object. Only its last value survives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicateKey {
    /// JSON Pointer to the object holding the key (`""` for the root).
    pub object: String,
    /// The repeated key.
    pub key: String,
}

impl Document {
    /// Parses JSON text.
    ///
    /// # Errors
    ///
    /// Fails only when the text is not JSON at all. A well-formed document of the wrong shape
    /// parses, and validation then explains what is wrong with it.
    pub fn from_json_str(text: &str) -> Result<Self, serde_json::Error> {
        let mut deserializer = serde_json::Deserializer::from_str(text);
        let document = Self::deserialize(&mut deserializer)?;
        deserializer.end()?;
        Ok(document)
    }

    /// Wraps a value that has already been parsed. Duplicate keys, if there were any, are already
    /// lost and cannot be reported.
    #[must_use]
    pub const fn from_value(value: Value) -> Self {
        Self {
            value,
            duplicates: Vec::new(),
        }
    }

    /// The document as a JSON value.
    #[must_use]
    pub const fn value(&self) -> &Value {
        &self.value
    }

    /// Mutable access to the value, for a caller that fills in defaults (such as the model) before
    /// validating.
    pub const fn value_mut(&mut self) -> &mut Value {
        &mut self.value
    }

    /// Every key that was repeated within one object, in document order.
    #[must_use]
    pub fn duplicate_keys(&self) -> &[DuplicateKey] {
        &self.duplicates
    }

    /// Converts the document into a typed request, once validation has passed.
    ///
    /// # Errors
    ///
    /// Fails when the document does not have the shape of a request, which validation reports in
    /// more useful terms.
    pub fn into_request(self) -> Result<Request, serde_json::Error> {
        serde_json::from_value(self.value)
    }
}

impl From<&Request> for Document {
    /// A typed request cannot hold duplicate keys, so the document has none. A request that
    /// somehow fails to serialise becomes `null`, which validation rejects.
    fn from(request: &Request) -> Self {
        Self::from_value(serde_json::to_value(request).unwrap_or(Value::Null))
    }
}

impl Serialize for Document {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Document {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut tracker = Tracker::default();
        let value = Node(&mut tracker).deserialize(deserializer)?;
        Ok(Self {
            value,
            duplicates: tracker.duplicates,
        })
    }
}

/// Appends `token` to a JSON Pointer, escaping it as RFC 6901 requires.
pub(crate) fn push_pointer(pointer: &mut String, token: &str) {
    pointer.push('/');
    for character in token.chars() {
        match character {
            '~' => pointer.push_str("~0"),
            '/' => pointer.push_str("~1"),
            other => pointer.push(other),
        }
    }
}

/// `base` extended by one token.
pub(crate) fn child_pointer(base: &str, token: &str) -> String {
    let mut pointer = base.to_owned();
    push_pointer(&mut pointer, token);
    pointer
}

#[derive(Default)]
struct Tracker {
    path: Vec<String>,
    duplicates: Vec<DuplicateKey>,
}

impl Tracker {
    fn pointer(&self) -> String {
        let mut pointer = String::new();
        for token in &self.path {
            push_pointer(&mut pointer, token);
        }
        pointer
    }
}

/// Builds a [`Value`] for one node of the document, recording duplicate keys on the way.
struct Node<'a>(&'a mut Tracker);

impl<'de> DeserializeSeed<'de> for Node<'_> {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Node<'_> {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Value, E> {
        Ok(Number::from_f64(value).map_or(Value::Null, Value::Number))
    }

    fn visit_str<E>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        loop {
            self.0.path.push(items.len().to_string());
            let item = access.next_element_seed(Node(&mut *self.0));
            self.0.path.pop();
            match item? {
                Some(item) => items.push(item),
                None => return Ok(Value::Array(items)),
            }
        }
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        let mut map = Map::new();
        while let Some(key) = access.next_key::<Key>()? {
            let Key(key) = key;
            if map.contains_key(&key) {
                let object = self.0.pointer();
                self.0.duplicates.push(DuplicateKey {
                    object,
                    key: key.clone(),
                });
            }
            self.0.path.push(key.clone());
            let value = access.next_value_seed(Node(&mut *self.0));
            self.0.path.pop();
            // Like `serde_json`, the last value wins, so the document matches what would be sent.
            map.insert(key, value?);
        }
        Ok(Value::Object(map))
    }
}

/// A map key. JSON keys are strings; YAML allows scalars, which are read as their text.
struct Key(String);

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KeyVisitor;

        impl Visitor<'_> for KeyVisitor {
            type Value = Key;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a string, number or boolean key")
            }

            fn visit_str<E>(self, value: &str) -> Result<Key, E> {
                Ok(Key(value.to_owned()))
            }

            fn visit_bool<E>(self, value: bool) -> Result<Key, E> {
                Ok(Key(value.to_string()))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Key, E> {
                Ok(Key(value.to_string()))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Key, E> {
                Ok(Key(value.to_string()))
            }

            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Key, E> {
                Err(E::custom(format_args!(
                    "a floating-point key ({value}) is not supported"
                )))
            }
        }

        deserializer.deserialize_any(KeyVisitor)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Document, DuplicateKey, child_pointer};
    use crate::{Noul, Request};

    #[test]
    fn keeps_the_whole_value_in_document_order() {
        let text = r#"{"z": 1, "a": [true, null, 2.5, "s"], "m": {"k": -3}}"#;

        let document = Document::from_json_str(text).unwrap();

        assert_eq!(
            document.value(),
            &json!({ "z": 1, "a": [true, null, 2.5, "s"], "m": { "k": -3 } })
        );
        assert_eq!(
            serde_json::to_string(&document).unwrap(),
            r#"{"z":1,"a":[true,null,2.5,"s"],"m":{"k":-3}}"#
        );
        assert!(document.duplicate_keys().is_empty());
    }

    #[test]
    fn records_every_duplicate_key_with_the_pointer_of_its_object() {
        let text = r#"{
            "questions": {
                "q": {"type": "noul"},
                "a/b": {"criteria": {"x": 1, "x": 2, "x": 3}},
                "q": {"type": "choice"}
            },
            "state": [{"k": 1, "k": 2}]
        }"#;

        let document = Document::from_json_str(text).unwrap();

        let duplicate = |object: &str, key: &str| DuplicateKey {
            object: object.into(),
            key: key.into(),
        };
        assert_eq!(
            document.duplicate_keys(),
            [
                duplicate("/questions/a~1b/criteria", "x"),
                duplicate("/questions/a~1b/criteria", "x"),
                duplicate("/questions", "q"),
                duplicate("/state/0", "k"),
            ]
        );
        assert_eq!(
            document.value()["questions"]["q"]["type"],
            "choice",
            "the last value wins"
        );
    }

    #[test]
    fn only_text_that_is_not_json_fails_to_parse() {
        assert!(Document::from_json_str("").is_err());
        assert!(Document::from_json_str("{").is_err());
        assert!(Document::from_json_str("{} trailing").is_err());

        for wrong_shape in ["[]", "null", "42", r#""text""#, r#"{"questions": 7}"#] {
            assert!(
                Document::from_json_str(wrong_shape).is_ok(),
                "{wrong_shape}"
            );
        }
    }

    #[test]
    fn a_typed_request_round_trips_through_a_document() {
        let request = Request::new("s", "jev-latest").question("q", Noul::new("?"));

        let document = Document::from(&request);

        assert!(document.duplicate_keys().is_empty());
        assert_eq!(document.clone().into_request().unwrap(), request);
        assert!(
            Document::from_value(json!({ "state": "s" }))
                .into_request()
                .is_err()
        );
    }

    #[test]
    fn the_value_can_be_completed_before_validation() {
        let mut document = Document::from_json_str(r#"{"state": "s", "questions": {}}"#).unwrap();

        document.value_mut()["model"] = json!("jev-latest");

        assert_eq!(document.value()["model"], "jev-latest");
    }

    #[test]
    fn deserialises_from_any_self_describing_source_not_only_json_text() {
        use serde::Deserialize;

        let source = json!({ "state": { "n": 1.5, "flag": true }, "questions": {} });

        let document = Document::deserialize(source.clone()).unwrap();

        assert_eq!(document.value(), &source);
    }

    #[test]
    fn pointers_escape_slashes_and_tildes() {
        assert_eq!(child_pointer("", "a"), "/a");
        assert_eq!(child_pointer("/questions", "a/b~c"), "/questions/a~1b~0c");
    }
}
