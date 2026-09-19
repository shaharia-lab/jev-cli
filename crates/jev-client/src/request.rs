//! The body of `POST /v1/systemone`.

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::content::{Content, non_null_schema};
use crate::de;
use crate::question::Question;

/// An evaluation request: one `state`, judged by every question independently and in parallel.
///
/// ```
/// use jev_client::{Choice, Noul, Request};
///
/// let request = Request::new("Help! My payouts have been failing for 3 days.", "jev-latest")
///     .question("is_urgent", Noul::new("Does this convey urgency?"))
///     .question(
///         "department",
///         Choice::new("Which team should handle this?")
///             .option("billing", "Payments, invoicing, refunds")
///             .option("technical", "Bugs, outages, integrations")
///             .bare_option("other"),
///     );
///
/// assert_eq!(request.questions.len(), 2);
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(
    description = "An evaluation request: one `state`, judged by every question independently and in parallel."
)]
#[non_exhaustive]
pub struct Request {
    /// The content to evaluate: a string, or structured data such as a chat log or a record.
    ///
    /// The API rejects `null` and scalar values here.
    #[schemars(schema_with = "non_null_schema")]
    pub state: Content,

    /// The model that handles the request: an alias such as `jev-latest`, or a versioned id such
    /// as `jev-1.13.0`. Aliases move without notice; pin a versioned id once thresholds are tuned.
    pub model: String,

    /// The questions, keyed by an id the caller chooses. Answers come back under the same ids.
    ///
    /// Questions keep the order they were written in. Ids are never sent to the model. A repeated
    /// id is a parse error when reading JSON or YAML text directly (a parser that has already
    /// collapsed the document into a generic value keeps only the last one).
    #[serde(deserialize_with = "de::unique_map")]
    pub questions: IndexMap<String, Question>,

    /// Top-level fields this version of the crate does not know, sent to the API unchanged.
    ///
    /// The API currently rejects any unknown top-level field with an uninformative
    /// `400 Invalid request.`, so validation reports these before a request is spent.
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

impl Request {
    /// A request for `model` over `state`, with no questions yet.
    #[must_use]
    pub fn new(state: impl Into<Content>, model: impl Into<String>) -> Self {
        Self {
            state: state.into(),
            model: model.into(),
            questions: IndexMap::new(),
            extra: IndexMap::new(),
        }
    }

    /// Adds a question under `id`, replacing any question already using that id.
    #[must_use]
    pub fn question(mut self, id: impl Into<String>, question: impl Into<Question>) -> Self {
        self.questions.insert(id.into(), question.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::Request;
    use crate::{Noul, Question};

    #[test]
    fn questions_keep_the_order_they_were_written_in() {
        let text = r#"{"state":"s","model":"jev-latest","questions":{"z":{"type":"noul","instructions":"?"},"a":{"type":"noul","instructions":"?"}}}"#;

        let request: Request = serde_json::from_str(text).unwrap();

        assert_eq!(request.questions.keys().collect::<Vec<_>>(), ["z", "a"]);
        assert_eq!(serde_json::to_string(&request).unwrap(), text);
    }

    #[test]
    fn a_repeated_question_id_is_a_parse_error() {
        let text = r#"{"state":"s","model":"m","questions":{"q":{"type":"noul","instructions":"a"},"q":{"type":"noul","instructions":"b"}}}"#;

        let error = serde_json::from_str::<Request>(text)
            .unwrap_err()
            .to_string();

        assert!(error.contains("duplicate key `q`"), "{error}");
    }

    #[test]
    fn the_builder_replaces_a_question_that_reuses_an_id() {
        let request = Request::new("s", "m")
            .question("q", Noul::new("first"))
            .question("q", Noul::new("second"));

        assert_eq!(request.questions.len(), 1);
        assert_eq!(
            request.questions.get("q"),
            Some(&Question::from(Noul::new("second")))
        );
    }

    #[test]
    fn unknown_top_level_fields_are_kept() {
        let json = json!({ "state": ["a"], "model": "m", "questions": {}, "temperature": 0 });

        let request: Request = serde_json::from_value(json.clone()).unwrap();

        assert_eq!(request.extra.get("temperature"), Some(&json!(0)));
        assert_eq!(serde_json::to_value(&request).unwrap(), json);
    }

    #[test]
    fn required_fields_are_required() {
        let error = serde_json::from_value::<Request>(json!({ "state": "s", "questions": {} }));

        assert!(
            error
                .unwrap_err()
                .to_string()
                .contains("missing field `model`")
        );
    }
}
