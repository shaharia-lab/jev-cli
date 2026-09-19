//! Typed answers: [`NoulAnswer`], [`ChoiceAnswer`] and [`ScoreAnswer`].

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::content::Content;

/// The answer to one question. The `type` field mirrors the question's type.
///
/// Parsing never fails on an answer this version of the crate does not understand. An unknown
/// `type`, or a known type whose body has changed shape, becomes [`Answer::Unknown`] holding the
/// raw JSON, so an upstream addition cannot take down the other answers in the response.
///
/// ```
/// use jev_client::Answer;
///
/// let known: Answer = serde_json::from_str(r#"{"type":"noul","noul":0.92}"#).unwrap();
/// assert_eq!(known.as_noul().map(|answer| answer.noul), Some(0.92));
///
/// let newer: Answer = serde_json::from_str(r#"{"type":"rank","order":["b","a"]}"#).unwrap();
/// assert_eq!(newer.type_name(), Some("rank"));
/// assert!(matches!(newer, Answer::Unknown(_)));
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
#[schemars(
    description = "The answer to one question. The `type` field mirrors the question's type. An answer of a type this version does not understand is passed through as the raw JSON received."
)]
#[non_exhaustive]
pub enum Answer {
    /// The probability that the answer to a yes/no question is yes.
    Noul(NoulAnswer),
    /// The chosen option and the probability of every option.
    Choice(ChoiceAnswer),
    /// The probability-weighted level and the probability of every level.
    Score(ScoreAnswer),
    /// An answer this version of the crate does not understand, kept as the raw JSON received.
    #[serde(untagged)]
    Unknown(Value),
}

impl Answer {
    /// The wire name of the answer type, such as `noul`.
    ///
    /// For [`Answer::Unknown`] this is whatever the server sent, or `None` when the answer had no
    /// string `type` at all.
    #[must_use]
    pub fn type_name(&self) -> Option<&str> {
        match self {
            Self::Noul(_) => Some("noul"),
            Self::Choice(_) => Some("choice"),
            Self::Score(_) => Some("score"),
            Self::Unknown(raw) => raw.get("type").and_then(Value::as_str),
        }
    }

    /// The answer as a [`NoulAnswer`], when it is one.
    #[must_use]
    pub const fn as_noul(&self) -> Option<&NoulAnswer> {
        match self {
            Self::Noul(answer) => Some(answer),
            _ => None,
        }
    }

    /// The answer as a [`ChoiceAnswer`], when it is one.
    #[must_use]
    pub const fn as_choice(&self) -> Option<&ChoiceAnswer> {
        match self {
            Self::Choice(answer) => Some(answer),
            _ => None,
        }
    }

    /// The answer as a [`ScoreAnswer`], when it is one.
    #[must_use]
    pub const fn as_score(&self) -> Option<&ScoreAnswer> {
        match self {
            Self::Score(answer) => Some(answer),
            _ => None,
        }
    }
}

impl From<NoulAnswer> for Answer {
    fn from(answer: NoulAnswer) -> Self {
        Self::Noul(answer)
    }
}

impl From<ChoiceAnswer> for Answer {
    fn from(answer: ChoiceAnswer) -> Self {
        Self::Choice(answer)
    }
}

impl From<ScoreAnswer> for Answer {
    fn from(answer: ScoreAnswer) -> Self {
        Self::Score(answer)
    }
}

/// The answer to a [`Noul`](crate::Noul). It has no `confidence`: the probability is the signal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(
    description = "The answer to a noul. It has no `confidence`: the probability is the signal."
)]
#[non_exhaustive]
pub struct NoulAnswer {
    /// The probability of yes, from 0 (no) to 1 (yes).
    pub noul: f64,
}

impl NoulAnswer {
    /// A Noul answer with the given probability of yes.
    #[must_use]
    pub const fn new(noul: f64) -> Self {
        Self { noul }
    }
}

/// The answer to a [`Choice`](crate::Choice).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(description = "The answer to a choice.")]
#[non_exhaustive]
pub struct ChoiceAnswer {
    /// The highest-probability option.
    pub choice: String,

    /// Every option mapped to its probability. The values sum to 1, within rounding.
    ///
    /// The server does not return options in the order they were asked.
    pub probabilities: IndexMap<String, f64>,

    /// How certain the model is, from 0 to 1, derived from `probabilities`.
    pub confidence: f64,
}

impl ChoiceAnswer {
    /// A Choice answer.
    #[must_use]
    pub fn new(
        choice: impl Into<String>,
        probabilities: IndexMap<String, f64>,
        confidence: f64,
    ) -> Self {
        Self {
            choice: choice.into(),
            probabilities,
            confidence,
        }
    }
}

/// The answer to a [`Score`](crate::Score).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(description = "The answer to a score.")]
#[non_exhaustive]
pub struct ScoreAnswer {
    /// The probability-weighted level. It can land between two levels.
    pub score: f64,

    /// Each level, as a **string** key such as `"0"`, mapped back to its description.
    ///
    /// The API reference types the description as a string, but the server echoes whatever the
    /// question used for that level, including an object or an array.
    pub legend: IndexMap<String, Content>,

    /// Each level, as a **string** key matching `legend`, mapped to its probability.
    pub probabilities: IndexMap<String, f64>,

    /// How certain the model is, from 0 to 1, derived from `probabilities`.
    pub confidence: f64,
}

impl ScoreAnswer {
    /// A Score answer.
    #[must_use]
    pub const fn new(
        score: f64,
        legend: IndexMap<String, Content>,
        probabilities: IndexMap<String, f64>,
        confidence: f64,
    ) -> Self {
        Self {
            score,
            legend,
            probabilities,
            confidence,
        }
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;
    use serde_json::{Value, json};

    use super::{Answer, ChoiceAnswer, NoulAnswer, ScoreAnswer};
    use crate::Content;

    fn parse(json: &Value) -> Answer {
        serde_json::from_value(json.clone()).unwrap()
    }

    #[test]
    fn known_answers_parse_into_their_variant() {
        let choice = parse(&json!({
            "type": "choice",
            "choice": "technical",
            "probabilities": { "billing": 0.08, "technical": 0.85, "sales": 0.07 },
            "confidence": 0.82
        }));

        let choice = choice.as_choice().unwrap();
        assert_eq!(choice.choice, "technical");
        assert_eq!(choice.probabilities.get("technical"), Some(&0.85));
        assert_eq!(
            parse(&json!({ "type": "noul", "noul": 1 })),
            Answer::from(NoulAnswer::new(1.0)),
            "an integer probability is read as a float"
        );
    }

    #[test]
    fn score_levels_are_keyed_by_string_and_the_legend_may_be_structured() {
        let json = json!({
            "type": "score",
            "score": 1.2,
            "confidence": 0.68,
            "legend": { "0": "Calm", "1": { "what": "Frustrated" }, "2": ["Very angry"] },
            "probabilities": { "0": 0.01, "1": 0.78, "2": 0.21 }
        });

        let answer = parse(&json);

        let score = answer.as_score().unwrap();
        assert_eq!(
            score.probabilities.keys().collect::<Vec<_>>(),
            ["0", "1", "2"]
        );
        assert_eq!(
            score.legend.get("0").and_then(Content::as_str),
            Some("Calm")
        );
        assert_eq!(
            score.legend.get("1").unwrap().as_value(),
            &json!({ "what": "Frustrated" })
        );
    }

    #[test]
    fn an_unknown_type_keeps_its_raw_json() {
        let json = json!({ "type": "rank", "order": ["b", "a"], "confidence": 0.5 });

        let answer = parse(&json);

        assert_eq!(answer, Answer::Unknown(json.clone()));
        assert_eq!(answer.type_name(), Some("rank"));
        assert_eq!(serde_json::to_value(&answer).unwrap(), json);
    }

    #[test]
    fn a_known_type_with_a_changed_shape_degrades_to_unknown_instead_of_failing() {
        let json = json!({ "type": "noul", "probability": 0.9 });

        let answer = parse(&json);

        assert_eq!(answer, Answer::Unknown(json));
        assert_eq!(answer.type_name(), Some("noul"));
        assert!(answer.as_noul().is_none());
    }

    #[test]
    fn an_answer_without_a_type_is_still_kept() {
        for json in [json!({ "noul": 0.5 }), json!("surprise"), json!(null)] {
            let answer = parse(&json);

            assert_eq!(answer, Answer::Unknown(json));
            assert_eq!(answer.type_name(), None);
        }
    }

    #[test]
    fn extra_fields_on_a_known_answer_are_ignored() {
        let answer = parse(&json!({ "type": "noul", "noul": 0.4, "confidence": 0.2 }));

        assert_eq!(answer, Answer::from(NoulAnswer::new(0.4)));
    }

    #[test]
    fn constructed_answers_serialise_with_their_type_tag() {
        let probabilities = IndexMap::from([("a".to_owned(), 0.75), ("b".to_owned(), 0.25)]);
        let choice = Answer::from(ChoiceAnswer::new("a", probabilities.clone(), 0.6));
        let legend = IndexMap::from([("0".to_owned(), Content::from("low"))]);
        let score = Answer::from(ScoreAnswer::new(0.0, legend, IndexMap::new(), 1.0));

        assert_eq!(
            serde_json::to_value(&choice).unwrap(),
            json!({ "type": "choice", "choice": "a", "probabilities": probabilities, "confidence": 0.6 })
        );
        assert_eq!(score.type_name(), Some("score"));
        assert_eq!(
            serde_json::to_value(&score).unwrap()["legend"],
            json!({ "0": "low" })
        );
    }
}
