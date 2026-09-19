//! Typed questions: [`Noul`], [`Choice`] and [`Score`].

use indexmap::IndexMap;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::content::{Content, non_null_schema};
use crate::de;

/// One typed question. The `type` field selects the variant.
///
/// A question's id (its key in [`Request::questions`](crate::Request::questions)) is never sent
/// to the model, so the full meaning belongs in `instructions`.
///
/// ```
/// use jev_client::{Noul, Question};
///
/// let question = Question::from(Noul::new("Does this convey urgency?"));
/// assert_eq!(
///     serde_json::to_string(&question).unwrap(),
///     r#"{"type":"noul","instructions":"Does this convey urgency?"}"#
/// );
/// ```
// The schemas are stricter than the parser on purpose. Parsing keeps unknown fields so validation
// can report them; the schema is the authoring contract, so it rejects them outright and an editor
// or an agent sees a misspelt `criteria` immediately. `unevaluatedProperties` is used on the
// variants, rather than `additionalProperties` on the structs, because it sees through the `$ref`
// that sits beside the `type` tag.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
#[schemars(
    description = "One typed question. The `type` field selects the kind: `noul`, `choice` or `score`. The question id is never sent to the model, so put the full meaning in `instructions`."
)]
#[non_exhaustive]
pub enum Question {
    /// A yes/no question, answered with the probability of yes.
    #[schemars(extend("unevaluatedProperties" = false))]
    Noul(Noul),
    /// Pick one option from a set, answered with a probability per option.
    #[schemars(extend("unevaluatedProperties" = false))]
    Choice(Choice),
    /// Rate along an ordered rubric, answered with a probability per level.
    #[schemars(extend("unevaluatedProperties" = false))]
    Score(Score),
}

impl Question {
    /// The wire name of the question type: `noul`, `choice` or `score`.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Noul(_) => "noul",
            Self::Choice(_) => "choice",
            Self::Score(_) => "score",
        }
    }

    /// The question's instructions, when it has any.
    #[must_use]
    pub const fn instructions(&self) -> Option<&Content> {
        match self {
            Self::Noul(question) => question.instructions.as_ref(),
            Self::Choice(question) => question.instructions.as_ref(),
            Self::Score(question) => question.instructions.as_ref(),
        }
    }

    /// Fields this version of the crate does not know, as written by the caller.
    ///
    /// The API silently accepts unknown fields inside a question, so a misspelt `criteria` would
    /// otherwise go unnoticed. Validation reads this to report them.
    #[must_use]
    pub const fn extra(&self) -> &IndexMap<String, Value> {
        match self {
            Self::Noul(question) => &question.extra,
            Self::Choice(question) => &question.extra,
            Self::Score(question) => &question.extra,
        }
    }
}

impl From<Noul> for Question {
    fn from(question: Noul) -> Self {
        Self::Noul(question)
    }
}

impl From<Choice> for Question {
    fn from(question: Choice) -> Self {
        Self::Choice(question)
    }
}

impl From<Score> for Question {
    fn from(question: Score) -> Self {
        Self::Score(question)
    }
}

/// A yes/no question. The answer is the probability that the answer is yes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
// `instructions` has a serde default so that validation, not parsing, reports its absence. The
// schema still requires it, as the API reference does.
#[schemars(extend("required" = ["instructions"]))]
#[non_exhaustive]
pub struct Noul {
    /// The yes/no question to evaluate.
    ///
    /// The API reference marks this as required. In practice the server also accepts a Noul that
    /// has only `criteria`, so it is optional here and offline validation decides.
    #[serde(
        default,
        deserialize_with = "de::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub instructions: Option<Content>,

    /// Optional descriptions of what a yes and a no mean.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criteria: Option<NoulCriteria>,

    /// Fields this version of the crate does not know, sent to the API unchanged.
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: IndexMap<String, Value>,
}

impl Noul {
    /// A Noul with the given instructions and no criteria.
    #[must_use]
    pub fn new(instructions: impl Into<Content>) -> Self {
        Self {
            instructions: Some(instructions.into()),
            ..Self::default()
        }
    }

    /// Sets what a yes and a no mean.
    #[must_use]
    pub fn with_criteria(mut self, criteria: NoulCriteria) -> Self {
        self.criteria = Some(criteria);
        self
    }
}

/// What a yes and a no mean for a [`Noul`]. Either side may be omitted.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(
    description = "What a yes and a no mean for a noul. Either side may be omitted.",
    extend("additionalProperties" = false)
)]
#[non_exhaustive]
pub struct NoulCriteria {
    /// What a yes (a value near 1) means.
    #[serde(
        rename = "true",
        default,
        deserialize_with = "de::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub when_true: Option<Content>,

    /// What a no (a value near 0) means.
    #[serde(
        rename = "false",
        default,
        deserialize_with = "de::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub when_false: Option<Content>,

    /// Keys other than `true` and `false`, sent to the API unchanged.
    ///
    /// The API accepts them without complaint; validation reports them.
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: IndexMap<String, Value>,
}

impl NoulCriteria {
    /// Criteria describing both sides of the question.
    #[must_use]
    pub fn new(when_true: impl Into<Content>, when_false: impl Into<Content>) -> Self {
        Self {
            when_true: Some(when_true.into()),
            when_false: Some(when_false.into()),
            extra: IndexMap::new(),
        }
    }
}

/// Picks one option from a set the caller defines.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("required" = ["instructions", "criteria"]))]
#[non_exhaustive]
pub struct Choice {
    /// What the model should decide.
    ///
    /// Marked required by the API reference, although the server accepts a Choice without it.
    #[serde(
        default,
        deserialize_with = "de::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub instructions: Option<Content>,

    /// Each option mapped to its description, or to `null` when the name says enough.
    ///
    /// Options keep the order they were written in. The API allows up to 255 of them. A repeated
    /// option is a parse error when reading JSON or YAML text directly.
    #[serde(deserialize_with = "de::unique_map")]
    pub criteria: IndexMap<String, Content>,

    /// Fields this version of the crate does not know, sent to the API unchanged.
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: IndexMap<String, Value>,
}

impl Choice {
    /// A Choice with the given instructions and no options yet.
    #[must_use]
    pub fn new(instructions: impl Into<Content>) -> Self {
        Self {
            instructions: Some(instructions.into()),
            ..Self::default()
        }
    }

    /// Adds an option with a description, replacing any option already using that name.
    #[must_use]
    pub fn option(mut self, name: impl Into<String>, description: impl Into<Content>) -> Self {
        self.criteria.insert(name.into(), description.into());
        self
    }

    /// Adds an option whose name needs no description (sent as `null`).
    #[must_use]
    pub fn bare_option(self, name: impl Into<String>) -> Self {
        self.option(name, Content::null())
    }
}

/// Rates the state along an ordered rubric. A level's number is its index in `criteria`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(extend("required" = ["instructions", "criteria"]))]
#[non_exhaustive]
pub struct Score {
    /// What the model should rate.
    ///
    /// Marked required by the API reference, although the server accepts a Score without it.
    #[serde(
        default,
        deserialize_with = "de::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub instructions: Option<Content>,

    /// Level descriptions in order, from level 0 upwards. The API allows 2 to 10 levels.
    ///
    /// Unlike a choice option, a level cannot be `null`: the server rejects it with a 422.
    #[schemars(schema_with = "levels_schema")]
    pub criteria: Vec<Content>,

    /// Fields this version of the crate does not know, sent to the API unchanged.
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: IndexMap<String, Value>,
}

impl Score {
    /// A Score with the given instructions and no levels yet.
    #[must_use]
    pub fn new(instructions: impl Into<Content>) -> Self {
        Self {
            instructions: Some(instructions.into()),
            ..Self::default()
        }
    }

    /// Appends the next level of the rubric.
    #[must_use]
    pub fn level(mut self, description: impl Into<Content>) -> Self {
        self.criteria.push(description.into());
        self
    }
}

/// Schema for [`Score::criteria`]: an array whose entries may not be `null`.
fn levels_schema(generator: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "array", "items": non_null_schema(generator) })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{Choice, Noul, NoulCriteria, Question, Score};
    use crate::Content;

    fn round_trip(json: &Value) -> Value {
        let question: Question = serde_json::from_value(json.clone()).unwrap();
        serde_json::to_value(&question).unwrap()
    }

    #[test]
    fn builders_produce_the_documented_wire_shape() {
        let noul = Noul::new("Does this convey urgency?").with_criteria(NoulCriteria::new(
            "Explicitly time-sensitive",
            "No urgency expressed",
        ));
        let choice = Choice::new("Which team should handle this?")
            .option("billing", "Payments, invoicing, refunds")
            .bare_option("other");
        let score = Score::new("How frustrated is the customer?")
            .level("Calm")
            .level("Angry");

        assert_eq!(
            serde_json::to_value(Question::from(noul)).unwrap(),
            json!({
                "type": "noul",
                "instructions": "Does this convey urgency?",
                "criteria": { "true": "Explicitly time-sensitive", "false": "No urgency expressed" }
            })
        );
        assert_eq!(
            serde_json::to_value(Question::from(choice)).unwrap(),
            json!({
                "type": "choice",
                "instructions": "Which team should handle this?",
                "criteria": { "billing": "Payments, invoicing, refunds", "other": null }
            })
        );
        assert_eq!(
            serde_json::to_value(Question::from(score)).unwrap(),
            json!({
                "type": "score",
                "instructions": "How frustrated is the customer?",
                "criteria": ["Calm", "Angry"]
            })
        );
    }

    #[test]
    fn structured_instructions_and_criteria_round_trip() {
        let json = json!({
            "type": "score",
            "instructions": { "question": "How risky is this change?", "focus": ["scope"] },
            "criteria": ["Safe", { "what": "Risky", "signals": ["touches billing"] }, ["Dangerous"]]
        });

        assert_eq!(round_trip(&json), json);
    }

    #[test]
    fn an_explicit_null_is_kept_distinct_from_an_absent_key() {
        let explicit =
            json!({ "type": "noul", "instructions": null, "criteria": { "true": null } });
        let absent = json!({ "type": "noul", "criteria": {} });

        assert_eq!(round_trip(&explicit), explicit);
        assert_eq!(round_trip(&absent), absent);

        let Question::Noul(noul) = serde_json::from_value(explicit).unwrap() else {
            panic!("expected a noul");
        };
        assert_eq!(noul.instructions, Some(Content::null()));
        let criteria = noul.criteria.unwrap();
        assert_eq!(criteria.when_true, Some(Content::null()));
        assert_eq!(criteria.when_false, None);
    }

    #[test]
    fn choice_options_keep_the_order_they_were_written_in() {
        let text = r#"{"type":"choice","instructions":"?","criteria":{"zebra":null,"apple":"a","mango":null}}"#;
        let question: Question = serde_json::from_str(text).unwrap();

        let Question::Choice(choice) = &question else {
            panic!("expected a choice")
        };
        assert_eq!(
            choice.criteria.keys().collect::<Vec<_>>(),
            ["zebra", "apple", "mango"]
        );
        assert_eq!(serde_json::to_string(&question).unwrap(), text);
    }

    #[test]
    fn a_repeated_choice_option_is_a_parse_error() {
        let text =
            r#"{"type":"choice","instructions":"?","criteria":{"a":null,"b":null,"a":"again"}}"#;

        let error = serde_json::from_str::<Question>(text)
            .unwrap_err()
            .to_string();

        assert!(error.contains("duplicate key `a`"), "{error}");
    }

    #[test]
    fn unknown_fields_are_kept_for_validation_and_sent_unchanged() {
        let json = json!({
            "type": "noul",
            "instructions": "Is it urgent?",
            "critera": { "true": "typo in the key above" },
            "criteria": { "true": "yes", "maybe": "not a real side" }
        });

        let question: Question = serde_json::from_value(json.clone()).unwrap();

        assert_eq!(question.extra().keys().collect::<Vec<_>>(), ["critera"]);
        let Question::Noul(noul) = &question else {
            panic!("expected a noul")
        };
        assert_eq!(
            noul.criteria
                .as_ref()
                .unwrap()
                .extra
                .keys()
                .collect::<Vec<_>>(),
            ["maybe"]
        );
        assert_eq!(serde_json::to_value(&question).unwrap(), json);
    }

    #[test]
    fn a_missing_or_unknown_type_is_rejected() {
        let missing = serde_json::from_value::<Question>(json!({ "instructions": "?" }));
        let unknown = serde_json::from_value::<Question>(json!({ "type": "rank" }));

        assert!(
            missing
                .unwrap_err()
                .to_string()
                .contains("missing field `type`")
        );
        assert!(
            unknown
                .unwrap_err()
                .to_string()
                .contains("unknown variant `rank`")
        );
    }

    #[test]
    fn accessors_report_the_type_and_instructions() {
        let question = Question::from(Score::new("Rate it").level("low").level("high"));

        assert_eq!(question.type_name(), "score");
        assert_eq!(
            question.instructions().and_then(Content::as_str),
            Some("Rate it")
        );
        assert!(question.extra().is_empty());
    }
}
