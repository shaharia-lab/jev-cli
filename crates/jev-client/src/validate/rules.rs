//! Shape, limits and size: the rules whose findings are errors.

use serde_json::{Map, Value};

use super::document::{Document, child_pointer};
use super::finding::{Finding, Rule};
use super::size::SizeEstimate;
use super::{MAX_CHOICE_OPTIONS, MAX_SCORE_LEVELS, MIN_SCORE_LEVELS, Options};

const REQUEST_FIELDS: [&str; 3] = ["state", "model", "questions"];
const QUESTION_FIELDS: [&str; 3] = ["type", "instructions", "criteria"];
const QUESTION_TYPES: [&str; 3] = ["noul", "choice", "score"];
const NOUL_CRITERIA_KEYS: [&str; 2] = ["true", "false"];

/// The estimate is reported from this fraction of a budget upwards...
const NEAR_BUDGET_PERCENT: u64 = 90;

/// ...and is an error from this fraction upwards. The estimate is good to about 15%, so a request
/// is only refused when it is clearly too large; between the two, the warning says it may be.
const OVER_BUDGET_PERCENT: u64 = 120;

/// The parts of a well-enough-formed request that the lints and the size estimate work on.
pub(super) struct RequestView<'a> {
    /// `None` when `state` is missing or has the wrong type, which has already been reported.
    pub(super) state: Option<&'a Value>,
    pub(super) questions: Vec<QuestionView<'a>>,
}

/// One question that is at least a JSON object.
pub(super) struct QuestionView<'a> {
    pub(super) id: &'a str,
    pub(super) pointer: String,
    pub(super) kind: Option<&'a str>,
    pub(super) value: &'a Value,
    pub(super) fields: &'a Map<String, Value>,
}

/// Checks shape and limits. Returns a view of the request unless it is not even an object.
pub(super) fn check_shape<'a>(
    document: &'a Document,
    options: &Options,
    findings: &mut Vec<Finding>,
) -> Option<RequestView<'a>> {
    let Value::Object(root) = document.value() else {
        findings.push(
            Finding::error(
                Rule::RequestShape,
                "",
                format!(
                    "a request must be a JSON object, not {}",
                    describe(document.value())
                ),
            )
            .suggest(r#"write {"state": ..., "model": "jev-latest", "questions": {...}}"#),
        );
        return None;
    };

    for key in root
        .keys()
        .filter(|key| !REQUEST_FIELDS.contains(&key.as_str()))
    {
        let mut finding = Finding::error(
            Rule::UnknownField,
            child_pointer("", key),
            format!("`{key}` is not a request field; the API rejects the whole request"),
        );
        finding = match closest(key, &REQUEST_FIELDS) {
            Some(known) => finding.suggest(format!("did you mean `{known}`?")),
            None => {
                finding.suggest("remove it; a request has only `state`, `model` and `questions`")
            }
        };
        findings.push(finding);
    }

    let state = check_state(root, options, findings);
    check_model(root, options, findings);
    let questions = check_questions(root, findings);
    check_duplicates(document, &questions, findings);

    Some(RequestView { state, questions })
}

fn check_state<'a>(
    root: &'a Map<String, Value>,
    options: &Options,
    findings: &mut Vec<Finding>,
) -> Option<&'a Value> {
    match root.get("state") {
        None if options.state_optional => None,
        None => {
            findings.push(
                Finding::error(Rule::StateMissing, "", "`state` is missing").suggest(
                    "add the content to evaluate as `state`: a string, an object or an array",
                ),
            );
            None
        }
        Some(state @ (Value::String(_) | Value::Object(_) | Value::Array(_))) => Some(state),
        Some(other) => {
            findings.push(
                Finding::error(
                    Rule::StateType,
                    "/state",
                    format!(
                        "`state` must be a string, an object or an array, not {}",
                        describe(other)
                    ),
                )
                .suggest("wrap a single value in a string or an object"),
            );
            None
        }
    }
}

fn check_model(root: &Map<String, Value>, options: &Options, findings: &mut Vec<Finding>) {
    match root.get("model") {
        None if options.model_optional => {}
        None => findings.push(
            Finding::error(Rule::ModelMissing, "", "`model` is missing")
                .suggest(r#"add "model": "jev-latest", or a versioned id such as "jev-1.13.0""#),
        ),
        Some(Value::String(model)) if !model.trim().is_empty() => {}
        Some(other) => findings.push(
            Finding::error(
                Rule::ModelType,
                "/model",
                format!(
                    "`model` must be a non-empty string, not {}",
                    describe(other)
                ),
            )
            .suggest(r#"use a model name such as "jev-latest""#),
        ),
    }
}

fn check_questions<'a>(
    root: &'a Map<String, Value>,
    findings: &mut Vec<Finding>,
) -> Vec<QuestionView<'a>> {
    let questions = match root.get("questions") {
        None => {
            findings.push(
                Finding::error(Rule::QuestionsMissing, "", "`questions` is missing").suggest(
                    r#"add "questions": {"<id>": {"type": "noul", "instructions": "..."}}"#,
                ),
            );
            return Vec::new();
        }
        Some(Value::Object(questions)) => questions,
        Some(other) => {
            findings.push(
                Finding::error(
                    Rule::QuestionsType,
                    "/questions",
                    format!(
                        "`questions` must be an object keyed by question id, not {}",
                        describe(other)
                    ),
                )
                .suggest("give every question an id and use it as the key"),
            );
            return Vec::new();
        }
    };
    if questions.is_empty() {
        findings.push(
            Finding::error(Rule::QuestionsEmpty, "/questions", "`questions` is empty")
                .suggest("add at least one question"),
        );
    }

    let mut views = Vec::new();
    for (id, value) in questions {
        let pointer = child_pointer("/questions", id);
        if id.trim().is_empty() {
            findings.push(
                Finding::error(
                    Rule::QuestionIdEmpty,
                    pointer.clone(),
                    "a question id is empty",
                )
                .in_question(id)
                .suggest("name the question; the answer comes back under the same id"),
            );
        }
        let Value::Object(fields) = value else {
            findings.push(
                Finding::error(
                    Rule::QuestionShape,
                    pointer,
                    format!("question `{id}` must be an object, not {}", describe(value)),
                )
                .in_question(id)
                .suggest(r#"write {"type": "noul", "instructions": "..."}"#),
            );
            continue;
        };
        let kind = check_question(id, &pointer, fields, findings);
        views.push(QuestionView {
            id,
            pointer,
            kind,
            value,
            fields,
        });
    }
    views
}

/// Checks one question and returns its type when it is a known one.
fn check_question<'a>(
    id: &str,
    pointer: &str,
    fields: &'a Map<String, Value>,
    findings: &mut Vec<Finding>,
) -> Option<&'a str> {
    for key in fields
        .keys()
        .filter(|key| !QUESTION_FIELDS.contains(&key.as_str()))
    {
        let mut finding = Finding::warning(
            Rule::UnknownField,
            child_pointer(pointer, key),
            format!("`{key}` is not a question field; the API ignores it without saying so"),
        )
        .in_question(id);
        finding = match closest(key, &QUESTION_FIELDS) {
            Some(known) => finding.suggest(format!("did you mean `{known}`?")),
            None => finding
                .suggest("remove it; a question has only `type`, `instructions` and `criteria`"),
        };
        findings.push(finding);
    }

    check_instructions(id, pointer, fields, findings);

    let kind = match fields.get("type") {
        None => {
            findings.push(
                Finding::error(
                    Rule::TypeMissing,
                    pointer,
                    format!("question `{id}` has no `type`"),
                )
                .in_question(id)
                .suggest(r#"add "type": "noul", "choice" or "score""#),
            );
            return None;
        }
        Some(Value::String(kind)) if QUESTION_TYPES.contains(&kind.as_str()) => kind.as_str(),
        Some(other) => {
            let shown = other
                .as_str()
                .map_or_else(|| describe(other).to_owned(), |text| format!("`{text}`"));
            let mut finding = Finding::error(
                Rule::TypeUnknown,
                child_pointer(pointer, "type"),
                format!(
                    "question `{id}` has type {shown}, which is not `noul`, `choice` or `score`"
                ),
            )
            .in_question(id);
            finding = match other
                .as_str()
                .and_then(|text| closest(&text.to_lowercase(), &QUESTION_TYPES))
            {
                Some(known) => finding.suggest(format!("did you mean `{known}`?")),
                None => finding
                    .suggest("use `noul` for yes/no, `choice` to pick one option, `score` to rate"),
            };
            findings.push(finding);
            return None;
        }
    };

    let criteria_pointer = child_pointer(pointer, "criteria");
    match kind {
        "noul" => check_noul_criteria(id, &criteria_pointer, fields.get("criteria"), findings),
        "choice" => check_choice_criteria(id, &criteria_pointer, fields.get("criteria"), findings),
        _ => check_score_criteria(id, &criteria_pointer, fields.get("criteria"), findings),
    }
    Some(kind)
}

fn check_instructions(
    id: &str,
    pointer: &str,
    fields: &Map<String, Value>,
    findings: &mut Vec<Finding>,
) {
    let instructions_pointer = child_pointer(pointer, "instructions");
    let empty = |value: &Value| match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Object(map) => map.is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    };
    match fields.get("instructions") {
        Some(value) if !empty(value) && !matches!(value, Value::Number(_) | Value::Bool(_)) => {}
        Some(value @ (Value::Number(_) | Value::Bool(_))) => findings.push(
            Finding::error(
                Rule::InstructionsType,
                instructions_pointer,
                format!(
                    "`instructions` must be a string, an object or an array, not {}",
                    describe(value)
                ),
            )
            .in_question(id)
            .suggest("write the question as text"),
        ),
        _ => findings.push(
            Finding::error(
                Rule::InstructionsMissing,
                instructions_pointer,
                format!("question `{id}` has no `instructions`"),
            )
            .in_question(id)
            .suggest("say what to decide; the question id is never shown to the model"),
        ),
    }
}

fn check_noul_criteria(
    id: &str,
    pointer: &str,
    criteria: Option<&Value>,
    findings: &mut Vec<Finding>,
) {
    let criteria = match criteria {
        None | Some(Value::Null) => return,
        Some(Value::Object(criteria)) => criteria,
        Some(other) => {
            findings.push(
                Finding::error(
                    Rule::CriteriaType,
                    pointer,
                    format!(
                        "noul `criteria` must be an object with `true` and `false`, not {}",
                        describe(other)
                    ),
                )
                .in_question(id)
                .suggest(
                    r#"write {"true": "what yes means", "false": "what no means"}, or remove it"#,
                ),
            );
            return;
        }
    };
    for (key, value) in criteria {
        let key_pointer = child_pointer(pointer, key);
        if !NOUL_CRITERIA_KEYS.contains(&key.as_str()) {
            findings.push(
                Finding::error(
                    Rule::NoulCriteriaKey,
                    key_pointer.clone(),
                    format!("noul `criteria` may only have `true` and `false`, not `{key}`"),
                )
                .in_question(id)
                .suggest("describe yes under `true` and no under `false`; use a choice for more than two outcomes"),
            );
        }
        check_content(id, &key_pointer, value, true, findings);
    }
}

fn check_choice_criteria(
    id: &str,
    pointer: &str,
    criteria: Option<&Value>,
    findings: &mut Vec<Finding>,
) {
    let options = match criteria {
        None | Some(Value::Null) => {
            findings.push(
                Finding::error(
                    Rule::CriteriaMissing,
                    pointer,
                    format!("choice `{id}` has no `criteria`"),
                )
                .in_question(id)
                .suggest(r#"list the options: {"billing": "Payments, refunds", "other": null}"#),
            );
            return;
        }
        Some(Value::Object(options)) => options,
        Some(other) => {
            findings.push(
                Finding::error(
                    Rule::CriteriaType,
                    pointer,
                    format!("choice `criteria` must be an object mapping each option to a description, not {}", describe(other)),
                )
                .in_question(id)
                .suggest(r#"write {"option": "description", "another": null}"#),
            );
            return;
        }
    };
    if options.is_empty() {
        findings.push(
            Finding::error(
                Rule::ChoiceNoOptions,
                pointer,
                format!("choice `{id}` has no options"),
            )
            .in_question(id)
            .suggest("add the options to choose between"),
        );
    }
    if options.len() > MAX_CHOICE_OPTIONS {
        findings.push(
            Finding::error(
                Rule::ChoiceTooManyOptions,
                pointer,
                format!(
                    "choice `{id}` has {} options; the limit is {MAX_CHOICE_OPTIONS}",
                    options.len()
                ),
            )
            .in_question(id)
            .suggest("split it into a hierarchy: choose a group first, then within the group"),
        );
    }
    for (name, description) in options {
        let option_pointer = child_pointer(pointer, name);
        if name.trim().is_empty() {
            findings.push(
                Finding::error(
                    Rule::ChoiceOptionEmpty,
                    option_pointer.clone(),
                    format!("choice `{id}` has an option with an empty name"),
                )
                .in_question(id)
                .suggest("name the option; the answer reports the name that was chosen"),
            );
        }
        check_content(id, &option_pointer, description, true, findings);
    }
}

fn check_score_criteria(
    id: &str,
    pointer: &str,
    criteria: Option<&Value>,
    findings: &mut Vec<Finding>,
) {
    let levels = match criteria {
        None | Some(Value::Null) => {
            findings.push(
                Finding::error(
                    Rule::CriteriaMissing,
                    pointer,
                    format!("score `{id}` has no `criteria`"),
                )
                .in_question(id)
                .suggest(r#"list the levels in order: ["Calm", "Frustrated", "Very angry"]"#),
            );
            return;
        }
        Some(Value::Array(levels)) => levels,
        Some(other) => {
            findings.push(
                Finding::error(
                    Rule::CriteriaType,
                    pointer,
                    format!("score `criteria` must be an ordered array of level descriptions, not {}", describe(other)),
                )
                .in_question(id)
                .suggest(r#"write the levels in order, lowest first: ["Calm", "Frustrated", "Very angry"]"#),
            );
            return;
        }
    };
    if levels.len() < MIN_SCORE_LEVELS {
        findings.push(
            Finding::error(
                Rule::ScoreTooFewLevels,
                pointer,
                format!(
                    "score `{id}` has {} level(s); at least {MIN_SCORE_LEVELS} are needed (the API accepts fewer and returns a meaningless answer)",
                    levels.len()
                ),
            )
            .in_question(id)
            .suggest("describe both ends of the scale, or use a noul for a yes/no question"),
        );
    }
    if levels.len() > MAX_SCORE_LEVELS {
        findings.push(
            Finding::error(
                Rule::ScoreTooManyLevels,
                pointer,
                format!(
                    "score `{id}` has {} levels; the limit is {MAX_SCORE_LEVELS} (the API fails with a server error beyond it)",
                    levels.len()
                ),
            )
            .in_question(id)
            .suggest("merge neighbouring levels; 3 to 5 well-separated levels usually work best"),
        );
    }
    for (index, level) in levels.iter().enumerate() {
        check_content(
            id,
            &child_pointer(pointer, &index.to_string()),
            level,
            false,
            findings,
        );
    }
}

/// Checks one criteria value: a string, an object or an array, and `null` where that is allowed.
fn check_content(
    id: &str,
    pointer: &str,
    value: &Value,
    null_allowed: bool,
    findings: &mut Vec<Finding>,
) {
    let problem = match value {
        Value::Number(_) | Value::Bool(_) => describe(value),
        Value::Null if !null_allowed => "null (the API rejects a null score level)",
        _ => return,
    };
    findings.push(
        Finding::error(
            Rule::CriteriaValueType,
            pointer,
            format!("a criteria value must be a string, an object or an array, not {problem}"),
        )
        .in_question(id)
        .suggest("describe it in words"),
    );
}

/// Turns the document's duplicate keys into findings.
fn check_duplicates(
    document: &Document,
    questions: &[QuestionView<'_>],
    findings: &mut Vec<Finding>,
) {
    for duplicate in document.duplicate_keys() {
        let pointer = child_pointer(&duplicate.object, &duplicate.key);
        let key = &duplicate.key;

        if duplicate.object == "/questions" {
            findings.push(
                Finding::error(
                    Rule::DuplicateQuestionId,
                    pointer,
                    format!("question id `{key}` is used more than once; only the last question would be sent"),
                )
                .in_question(key)
                .suggest("give every question its own id"),
            );
            continue;
        }

        let owner = questions.iter().find(|question| {
            duplicate.object == question.pointer
                || duplicate
                    .object
                    .starts_with(&format!("{}/", question.pointer))
        });
        let is_choice_option = owner.is_some_and(|question| {
            question.kind == Some("choice")
                && duplicate.object == child_pointer(&question.pointer, "criteria")
        });
        let mut finding = if is_choice_option {
            Finding::error(
                Rule::DuplicateOption,
                pointer,
                format!("option `{key}` is listed more than once; only the last description would be sent"),
            )
            .suggest("keep one entry per option")
        } else {
            Finding::warning(
                Rule::DuplicateKey,
                pointer,
                format!("key `{key}` is repeated in the same object; only the last value is sent"),
            )
            .suggest("remove the repeated key, or rename one of them")
        };
        if let Some(question) = owner {
            finding = finding.in_question(question.id);
        }
        findings.push(finding);
    }
}

/// Compares the estimate with both budgets.
pub(super) fn check_size(estimate: &SizeEstimate, findings: &mut Vec<Finding>) {
    let checks = [
        (
            Rule::SizeTotal,
            estimate.total_tokens,
            estimate.total_budget_tokens,
            "the whole request".to_owned(),
            "",
            None,
            "shorten `state`, or send the questions in several requests",
        ),
        (
            Rule::SizeQuestion,
            estimate.largest_question_tokens,
            estimate.question_budget_tokens,
            estimate.largest_question.as_ref().map_or_else(
                || "`state`".to_owned(),
                |id| format!("`state` plus the longest question (`{id}`)"),
            ),
            "/state",
            estimate.largest_question.as_deref(),
            "shorten or chunk `state`; every question is judged against the whole of it",
        ),
    ];

    for (rule, tokens, budget, what, path, question, suggestion) in checks {
        let percent = tokens.saturating_mul(100) / budget.max(1);
        if percent < NEAR_BUDGET_PERCENT {
            continue;
        }
        let numbers = format!("an estimated {tokens} tokens against a budget of {budget}");
        let mut finding = if percent >= OVER_BUDGET_PERCENT {
            Finding::error(
                rule,
                path,
                format!("{what} is too large: {numbers} (an estimate, good to about 15%)"),
            )
        } else if tokens > budget {
            Finding::warning(
                rule,
                path,
                format!("{what} may be too large: {numbers} (an estimate, good to about 15%)"),
            )
        } else {
            Finding::warning(
                rule,
                path,
                format!("{what} is close to the limit: {numbers} (an estimate, good to about 15%)"),
            )
        }
        .suggest(suggestion);
        if let Some(id) = question {
            finding = finding.in_question(id);
        }
        findings.push(finding);
    }
}

/// A short description of a JSON value's type, for messages.
fn describe(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// The known name that `written` is most likely a misspelling of, if any is close enough.
fn closest<'a>(written: &str, known: &[&'a str]) -> Option<&'a str> {
    let written = written.to_lowercase();
    known
        .iter()
        .map(|name| (*name, edit_distance(&written, name)))
        .filter(|(name, distance)| *distance <= 2.max(name.len() / 4))
        .min_by_key(|(_, distance)| *distance)
        .map(|(name, _)| name)
}

/// Levenshtein distance between two short strings.
fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (row, left_char) in left.chars().enumerate() {
        let mut current = vec![row + 1];
        for (column, right_char) in right.iter().enumerate() {
            let substitution = previous.get(column).copied().unwrap_or(usize::MAX)
                + usize::from(left_char != *right_char);
            let deletion = previous
                .get(column + 1)
                .copied()
                .unwrap_or(usize::MAX)
                .saturating_add(1);
            let insertion = current
                .last()
                .copied()
                .unwrap_or(usize::MAX)
                .saturating_add(1);
            current.push(substitution.min(deletion).min(insertion));
        }
        previous = current;
    }
    previous.last().copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{closest, edit_distance};

    #[test]
    fn edit_distance_counts_single_character_edits() {
        assert_eq!(edit_distance("criteria", "criteria"), 0);
        assert_eq!(edit_distance("critera", "criteria"), 1);
        assert_eq!(edit_distance("instruction", "instructions"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("abc", ""), 3);
    }

    #[test]
    fn suggests_a_field_only_when_the_spelling_is_close() {
        let fields = ["type", "instructions", "criteria"];

        assert_eq!(closest("critera", &fields), Some("criteria"));
        assert_eq!(closest("Instruction", &fields), Some("instructions"));
        assert_eq!(closest("tpye", &fields), Some("type"));
        assert_eq!(closest("temperature", &fields), None);
        assert_eq!(closest("x", &fields), None);
    }
}
