//! The body returned by `GET /v1/models`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The model names an account can send in a request's `model` field.
///
/// The endpoint currently lists aliases only. A versioned id such as `jev-1.13.0` is accepted by
/// the API whether or not it appears here.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ModelList {
    /// One entry per model or alias.
    #[serde(default)]
    pub models: Vec<ModelCard>,
}

impl ModelList {
    /// A list of the given models.
    #[must_use]
    pub const fn new(models: Vec<ModelCard>) -> Self {
        Self { models }
    }
}

/// One model or alias, as listed by the API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ModelCard {
    /// The model id or alias, as accepted by a request's `model` field.
    pub name: String,

    /// What the model is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// When the model or alias was released, as the RFC 3339 timestamp the API sent.
    ///
    /// Kept as text so that a change of format upstream cannot fail the whole listing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
}

impl ModelCard {
    /// A model card with only a name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            release_date: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ModelCard, ModelList};

    #[test]
    fn tolerates_missing_and_unknown_fields() {
        let json = json!({
            "models": [{ "name": "jev-latest", "context_length": 64000 }],
            "next_page": null
        });

        let list: ModelList = serde_json::from_value(json).unwrap();

        assert_eq!(list, ModelList::new(vec![ModelCard::new("jev-latest")]));
    }

    #[test]
    fn an_empty_body_is_an_empty_list() {
        let list: ModelList = serde_json::from_value(json!({})).unwrap();

        assert!(list.models.is_empty());
    }
}
