//! What validation reports: findings, the rules behind them, and the report that collects them.

use schemars::JsonSchema;
use serde::Serialize;

use super::size::SizeEstimate;

/// How serious a finding is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Severity {
    /// The request is probably fine to send, but something about it is likely to give worse
    /// answers. Strict mode turns warnings into errors.
    Warning,
    /// The request must not be sent: the API would reject it, or answer something other than what
    /// was asked.
    Error,
}

/// The rule a finding comes from. [`Rule::id`] is a stable, machine-readable name.
///
/// Rules fall into three groups: shape and limits (always errors), the size estimate, and lints
/// (advice from TypeSafe's design guidance, reported as warnings).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Rule {
    /// The request is not a JSON object.
    RequestShape,
    /// A field the API does not define. At the top level the API rejects it; inside a question the
    /// API silently ignores it, which usually means a misspelt `criteria` or `instructions`.
    UnknownField,
    /// `state` is absent.
    StateMissing,
    /// `state` is not a string, an object or an array.
    StateType,
    /// `model` is absent.
    ModelMissing,
    /// `model` is not a non-empty string.
    ModelType,
    /// `questions` is absent.
    QuestionsMissing,
    /// `questions` is not an object.
    QuestionsType,
    /// `questions` has no entries.
    QuestionsEmpty,
    /// A question id is empty.
    QuestionIdEmpty,
    /// The same question id is used twice, so only the last question would be sent.
    DuplicateQuestionId,
    /// A question is not a JSON object.
    QuestionShape,
    /// A question has no `type`.
    TypeMissing,
    /// A question's `type` is not `noul`, `choice` or `score`.
    TypeUnknown,
    /// A question has no `instructions`, or they are `null` or empty.
    InstructionsMissing,
    /// `instructions` is not a string, an object or an array.
    InstructionsType,
    /// A choice or a score has no `criteria`.
    CriteriaMissing,
    /// `criteria` has the wrong JSON type for the question.
    CriteriaType,
    /// A criteria value is a number or a boolean, or a score level is `null`.
    CriteriaValueType,
    /// Noul `criteria` has a key other than `true` or `false`.
    NoulCriteriaKey,
    /// A choice has no options.
    ChoiceNoOptions,
    /// A choice has more than 255 options.
    ChoiceTooManyOptions,
    /// A choice option has an empty name.
    ChoiceOptionEmpty,
    /// The same choice option is listed twice, so only the last description would be sent.
    DuplicateOption,
    /// A score has fewer than 2 levels. The API accepts this and returns a meaningless answer.
    ScoreTooFewLevels,
    /// A score has more than 10 levels. The API fails with a server error.
    ScoreTooManyLevels,
    /// The whole request is estimated to be near, or beyond, the 64k-token budget.
    SizeTotal,
    /// The state plus the longest question is estimated to be near, or beyond, the 32k-token budget.
    SizeQuestion,
    /// Lint: a choice offers no way out such as `other` or `not_stated`, so the model must pick
    /// an option even when none fits.
    ChoiceNoEscapeOption,
    /// Lint: a choice between yes and no. A noul is the better tool.
    ChoiceYesNo,
    /// Lint: a backticked path indexes into a long array, where the model is known to miscount.
    PathIndexLongArray,
    /// Lint: two options, or two score levels, have the same description.
    DuplicateDescription,
    /// Lint: a key is repeated inside `state` or structured content; only the last value is sent.
    DuplicateKey,
}

impl Rule {
    /// Every rule, in the order they are documented.
    pub const ALL: [Self; 33] = [
        Self::RequestShape,
        Self::UnknownField,
        Self::StateMissing,
        Self::StateType,
        Self::ModelMissing,
        Self::ModelType,
        Self::QuestionsMissing,
        Self::QuestionsType,
        Self::QuestionsEmpty,
        Self::QuestionIdEmpty,
        Self::DuplicateQuestionId,
        Self::QuestionShape,
        Self::TypeMissing,
        Self::TypeUnknown,
        Self::InstructionsMissing,
        Self::InstructionsType,
        Self::CriteriaMissing,
        Self::CriteriaType,
        Self::CriteriaValueType,
        Self::NoulCriteriaKey,
        Self::ChoiceNoOptions,
        Self::ChoiceTooManyOptions,
        Self::ChoiceOptionEmpty,
        Self::DuplicateOption,
        Self::ScoreTooFewLevels,
        Self::ScoreTooManyLevels,
        Self::SizeTotal,
        Self::SizeQuestion,
        Self::ChoiceNoEscapeOption,
        Self::ChoiceYesNo,
        Self::PathIndexLongArray,
        Self::DuplicateDescription,
        Self::DuplicateKey,
    ];

    /// The stable identifier of the rule, such as `score-too-many-levels`. It is the same text the
    /// rule serialises to.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::RequestShape => "request-shape",
            Self::UnknownField => "unknown-field",
            Self::StateMissing => "state-missing",
            Self::StateType => "state-type",
            Self::ModelMissing => "model-missing",
            Self::ModelType => "model-type",
            Self::QuestionsMissing => "questions-missing",
            Self::QuestionsType => "questions-type",
            Self::QuestionsEmpty => "questions-empty",
            Self::QuestionIdEmpty => "question-id-empty",
            Self::DuplicateQuestionId => "duplicate-question-id",
            Self::QuestionShape => "question-shape",
            Self::TypeMissing => "type-missing",
            Self::TypeUnknown => "type-unknown",
            Self::InstructionsMissing => "instructions-missing",
            Self::InstructionsType => "instructions-type",
            Self::CriteriaMissing => "criteria-missing",
            Self::CriteriaType => "criteria-type",
            Self::CriteriaValueType => "criteria-value-type",
            Self::NoulCriteriaKey => "noul-criteria-key",
            Self::ChoiceNoOptions => "choice-no-options",
            Self::ChoiceTooManyOptions => "choice-too-many-options",
            Self::ChoiceOptionEmpty => "choice-option-empty",
            Self::DuplicateOption => "duplicate-option",
            Self::ScoreTooFewLevels => "score-too-few-levels",
            Self::ScoreTooManyLevels => "score-too-many-levels",
            Self::SizeTotal => "size-total",
            Self::SizeQuestion => "size-question",
            Self::ChoiceNoEscapeOption => "choice-no-escape-option",
            Self::ChoiceYesNo => "choice-yes-no",
            Self::PathIndexLongArray => "path-index-long-array",
            Self::DuplicateDescription => "duplicate-description",
            Self::DuplicateKey => "duplicate-key",
        }
    }
}

/// One problem found in a request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[non_exhaustive]
pub struct Finding {
    /// How serious the problem is, after strict mode has been applied.
    pub severity: Severity,
    /// The rule that was broken.
    pub rule: Rule,
    /// The id of the question the problem is in, when it is in one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    /// A JSON Pointer (RFC 6901) to the offending part of the request, such as
    /// `/questions/frustration/criteria`. The empty string is the request itself.
    pub path: String,
    /// What is wrong.
    pub message: String,
    /// How to fix it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

impl Finding {
    pub(crate) fn error(rule: Rule, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, rule, path, message)
    }

    pub(crate) fn warning(rule: Rule, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, rule, path, message)
    }

    fn new(
        severity: Severity,
        rule: Rule,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            rule,
            question: None,
            path: path.into(),
            message: message.into(),
            suggestion: None,
        }
    }

    pub(crate) fn in_question(mut self, id: &str) -> Self {
        self.question = Some(id.to_owned());
        self
    }

    pub(crate) fn suggest(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }
}

/// The result of validating a request.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[non_exhaustive]
pub struct Report {
    /// Everything that was found, in document order, then lints, then size.
    pub findings: Vec<Finding>,
    /// The size estimate, unless the size check was skipped or the request had no usable shape.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<SizeEstimate>,
}

impl Report {
    /// Returns `true` when nothing stands in the way of sending the request: there may be
    /// warnings, but there are no errors.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.errors().next().is_none()
    }

    /// The findings that are errors.
    pub fn errors(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|finding| finding.severity == Severity::Error)
    }

    /// The findings that are warnings.
    pub fn warnings(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|finding| finding.severity == Severity::Warning)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::json;

    use super::{Finding, Rule, Severity};

    #[test]
    fn every_rule_is_listed_once_and_serialises_to_its_id() {
        let ids: HashSet<&str> = Rule::ALL.iter().map(|rule| rule.id()).collect();
        assert_eq!(
            ids.len(),
            Rule::ALL.len(),
            "rule ids are unique and ALL has no repeats"
        );

        for rule in Rule::ALL {
            assert_eq!(
                serde_json::to_value(rule).unwrap(),
                json!(rule.id()),
                "{rule:?}"
            );
            assert!(
                rule.id()
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '-'),
                "{}",
                rule.id()
            );
        }
    }

    #[test]
    fn a_finding_serialises_without_its_empty_fields() {
        let bare = Finding::error(Rule::StateMissing, "", "`state` is missing");
        let full = Finding::warning(Rule::ChoiceYesNo, "/questions/q/criteria", "yes/no choice")
            .in_question("q")
            .suggest("use a noul");

        assert_eq!(
            serde_json::to_value(&bare).unwrap(),
            json!({ "severity": "error", "rule": "state-missing", "path": "", "message": "`state` is missing" })
        );
        assert_eq!(
            serde_json::to_value(&full).unwrap(),
            json!({
                "severity": "warning",
                "rule": "choice-yes-no",
                "question": "q",
                "path": "/questions/q/criteria",
                "message": "yes/no choice",
                "suggestion": "use a noul"
            })
        );
        assert!(Severity::Error > Severity::Warning);
    }
}
