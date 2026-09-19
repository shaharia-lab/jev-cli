//! The body returned by `POST /v1/systemone`.

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::answer::Answer;

/// An evaluation response: one answer per question, under the ids the request used.
///
/// Parsing is tolerant so that an upstream addition never breaks a caller: unknown fields are
/// ignored, a missing `usage` reads as zero, and an answer of an unknown type is kept as raw JSON
/// (see [`Answer::Unknown`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(
    description = "An evaluation response: one answer per question, under the ids the request used."
)]
#[non_exhaustive]
pub struct Response {
    /// The versioned id of the model that answered, such as `jev-1.13.0`.
    ///
    /// This is the resolved id even when the request named an alias, so it is the value to log
    /// and the value pricing is keyed on.
    pub model: String,

    /// One answer per question, keyed by the ids used in the request.
    pub answers: IndexMap<String, Answer>,

    /// Token usage for the request.
    #[serde(default)]
    pub usage: Usage,
}

impl Response {
    /// A response, as a fake transport or a test would build one.
    #[must_use]
    pub fn new(model: impl Into<String>, answers: IndexMap<String, Answer>, usage: Usage) -> Self {
        Self {
            model: model.into(),
            answers,
            usage,
        }
    }
}

/// Token usage for one request. Only input tokens are billed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Usage {
    /// Tokens read: the state, every question, and a fixed per-request overhead.
    #[serde(default)]
    pub input_tokens: u64,

    /// Tokens produced. Reported by the API but not billed.
    #[serde(default)]
    pub output_tokens: u64,
}

impl Usage {
    /// Usage with the given token counts.
    #[must_use]
    pub const fn new(input_tokens: u64, output_tokens: u64) -> Self {
        Self {
            input_tokens,
            output_tokens,
        }
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;
    use serde_json::json;

    use super::{Response, Usage};
    use crate::{Answer, NoulAnswer};

    #[test]
    fn unknown_fields_are_ignored_and_unknown_answers_are_kept() {
        let rank = json!({ "type": "rank", "order": ["b", "a"] });
        let json = json!({
            "model": "jev-2.0.0",
            "warnings": ["state was truncated"],
            "answers": { "urgent": { "type": "noul", "noul": 0.9 }, "ranking": rank },
            "usage": { "input_tokens": 312, "output_tokens": 48, "cached_tokens": 100 }
        });

        let response: Response = serde_json::from_value(json).unwrap();

        assert_eq!(response.model, "jev-2.0.0");
        assert_eq!(response.usage, Usage::new(312, 48));
        assert_eq!(
            response.answers.get("urgent"),
            Some(&Answer::from(NoulAnswer::new(0.9)))
        );
        assert_eq!(
            response.answers.get("ranking"),
            Some(&Answer::Unknown(rank))
        );
    }

    #[test]
    fn missing_usage_reads_as_zero() {
        let whole: Response =
            serde_json::from_value(json!({ "model": "m", "answers": {} })).unwrap();
        let partial: Response = serde_json::from_value(
            json!({ "model": "m", "answers": {}, "usage": { "input_tokens": 7 } }),
        )
        .unwrap();

        assert_eq!(whole.usage, Usage::default());
        assert_eq!(partial.usage, Usage::new(7, 0));
    }

    #[test]
    fn answers_keep_the_order_the_server_sent() {
        let text = r#"{"model":"m","answers":{"z":{"type":"noul","noul":0.1},"a":{"type":"noul","noul":0.2}},"usage":{"input_tokens":1,"output_tokens":2}}"#;

        let response: Response = serde_json::from_str(text).unwrap();

        assert_eq!(response.answers.keys().collect::<Vec<_>>(), ["z", "a"]);
        assert_eq!(serde_json::to_string(&response).unwrap(), text);
    }

    #[test]
    fn a_response_can_be_built_for_fakes() {
        let answers = IndexMap::from([("q".to_owned(), Answer::from(NoulAnswer::new(0.5)))]);

        let response = Response::new("jev-1.13.0", answers, Usage::new(10, 0));

        assert_eq!(response.answers.len(), 1);
        assert_eq!(response.usage.input_tokens, 10);
    }
}
