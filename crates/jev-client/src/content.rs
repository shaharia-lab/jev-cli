//! Free-form JSON content: the shape shared by `state`, `instructions` and criteria values.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Free-form JSON that Jev reads as content: a string, an object, an array, or `null`.
///
/// The API accepts this shape for a request's `state`, for a question's `instructions`, and for
/// every criteria value. Field names inside an object (`question`, `focus`, `examples`, ...) are
/// not reserved: the model sees them as labels.
///
/// Deserialisation is deliberately permissive. A number or a boolean parses successfully so that
/// offline validation can report it with a useful message instead of a parse error; the generated
/// JSON Schema is the precise contract. Object key order is preserved, so content is sent to the
/// API exactly as it was written.
///
/// ```
/// use jev_client::Content;
///
/// let text = Content::from("Help! My payouts have been failing for 3 days.");
/// assert_eq!(text.as_str(), Some("Help! My payouts have been failing for 3 days."));
///
/// let structured = Content::from(serde_json::json!({ "ticket": { "plan": "pro" } }));
/// assert_eq!(structured.as_str(), None);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Content(Value);

impl Content {
    /// Content that is JSON `null`, used for a choice option that needs no description.
    #[must_use]
    pub const fn null() -> Self {
        Self(Value::Null)
    }

    /// Returns `true` when the content is JSON `null`.
    #[must_use]
    pub fn is_null(&self) -> bool {
        self.0.is_null()
    }

    /// The text, when the content is a plain string.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        self.0.as_str()
    }

    /// Borrows the underlying JSON value.
    #[must_use]
    pub const fn as_value(&self) -> &Value {
        &self.0
    }

    /// Consumes the content and returns the underlying JSON value.
    #[must_use]
    pub fn into_value(self) -> Value {
        self.0
    }
}

impl From<Value> for Content {
    fn from(value: Value) -> Self {
        Self(value)
    }
}

impl From<Content> for Value {
    fn from(content: Content) -> Self {
        content.0
    }
}

impl From<String> for Content {
    fn from(text: String) -> Self {
        Self(Value::String(text))
    }
}

impl From<&str> for Content {
    fn from(text: &str) -> Self {
        Self(Value::String(text.to_owned()))
    }
}

impl JsonSchema for Content {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "Content".into()
    }

    fn schema_id() -> Cow<'static, str> {
        concat!(module_path!(), "::Content").into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Free-form content: a string, an object, an array, or null.",
            "type": ["string", "object", "array", "null"]
        })
    }
}

/// Schema for content that may not be `null`: a request's `state` and each score level.
///
/// Live testing showed the API rejects `null` in both places with a 422, even though the upstream
/// documentation lists `null` as accepted for score levels.
pub(crate) fn non_null_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "description": "Free-form content: a string, an object, or an array.",
        "type": ["string", "object", "array"]
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::Content;

    #[test]
    fn serialises_transparently_as_the_wrapped_value() {
        let content = Content::from(json!({ "b": 1, "a": [true, null] }));

        assert_eq!(
            serde_json::to_string(&content).unwrap(),
            r#"{"b":1,"a":[true,null]}"#
        );
    }

    #[test]
    fn accepts_scalars_so_that_validation_can_report_them() {
        let content: Content = serde_json::from_str("42").unwrap();

        assert_eq!(content.as_value(), &json!(42));
        assert_eq!(content.as_str(), None);
    }

    #[test]
    fn distinguishes_null_from_text() {
        assert!(Content::null().is_null());
        assert!(Content::default().is_null());
        assert!(!Content::from("").is_null());
    }

    #[test]
    fn converts_to_and_from_json_values() {
        let value = json!(["a", { "k": "v" }]);

        assert_eq!(Content::from(value.clone()).into_value(), value);
        assert_eq!(serde_json::Value::from(Content::from("x")), json!("x"));
        assert_eq!(Content::from(String::from("x")), Content::from("x"));
    }
}
