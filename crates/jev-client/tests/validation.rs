//! Offline validation: every rule, the limits at their boundaries, strict mode, and the accuracy
//! of the size estimate against real token counts.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::collections::HashSet;

use jev_client::validate::{
    self, Document, Finding, Options, QUESTION_BUDGET_TOKENS, Report, Rule, Severity,
    TOTAL_BUDGET_TOKENS, estimate_value_tokens,
};
use jev_client::{Choice, Noul, Request, Score};
use serde_json::{Value, json};

fn check_text(text: &str) -> Report {
    validate::check_document(&Document::from_json_str(text).unwrap(), &Options::default())
}

fn check_value(value: &Value) -> Report {
    validate::check_document(&Document::from_value(value.clone()), &Options::default())
}

/// A request around one question called `q`.
fn with_question(question: &Value) -> Value {
    json!({ "state": "Help! My payouts have been failing.", "model": "jev-latest", "questions": { "q": question } })
}

fn only(report: &Report, rule: Rule) -> &Finding {
    let mut found = report
        .findings
        .iter()
        .filter(|finding| finding.rule == rule);
    match (found.next(), found.next()) {
        (Some(finding), None) => finding,
        _ => panic!(
            "expected exactly one `{}` finding in {:#?}",
            rule.id(),
            report.findings
        ),
    }
}

fn choice_with_options(count: usize) -> Value {
    let mut options: serde_json::Map<String, Value> = (0..count.saturating_sub(1))
        .map(|index| (format!("option_{index}"), Value::Null))
        .collect();
    options.insert("other".into(), Value::Null);
    json!({ "type": "choice", "instructions": "Which one?", "criteria": options })
}

fn score_with_levels(count: usize) -> Value {
    let levels: Vec<String> = (0..count).map(|level| format!("Level {level}")).collect();
    json!({ "type": "score", "instructions": "How much?", "criteria": levels })
}

#[test]
fn the_api_reference_examples_are_valid_and_clean() {
    for fixture in common::load("api-reference", "request-") {
        let report = check_value(&fixture.json);

        assert!(
            report.is_valid(),
            "{}: {:#?}",
            fixture.name,
            report.findings
        );
        let unexpected: Vec<&Finding> = report
            .findings
            .iter()
            .filter(|finding| finding.rule != Rule::ChoiceNoEscapeOption)
            .collect();
        assert!(unexpected.is_empty(), "{}: {unexpected:#?}", fixture.name);
        assert!(report.size.is_some());
    }
}

#[test]
fn a_request_the_live_api_accepted_is_valid() {
    for fixture in common::load("live", "request-") {
        let report = check_value(&fixture.json);

        assert_eq!(report.findings, [], "{}", fixture.name);
    }
}

#[test]
fn a_typed_request_is_checked_the_same_way() {
    let valid = Request::new("state", "jev-latest")
        .question("urgent", Noul::new("Is it urgent?"))
        .question(
            "team",
            Choice::new("Which team?")
                .option("billing", "Payments")
                .bare_option("other"),
        )
        .question(
            "anger",
            Score::new("How angry?").level("Calm").level("Furious"),
        );
    let invalid = Request::new("state", "jev-latest")
        .question("anger", Score::new("How angry?").level("Calm"));

    assert_eq!(validate::check(&valid, &Options::default()).findings, []);
    let report = validate::check(&invalid, &Options::default());
    assert_eq!(
        only(&report, Rule::ScoreTooFewLevels).question.as_deref(),
        Some("anger")
    );
}

#[test]
fn limits_are_rejected_with_the_question_id_and_the_rule() {
    let cases = [
        (score_with_levels(1), Rule::ScoreTooFewLevels, "1 level(s)"),
        (score_with_levels(0), Rule::ScoreTooFewLevels, "0 level(s)"),
        (score_with_levels(11), Rule::ScoreTooManyLevels, "11 levels"),
        (
            choice_with_options(256),
            Rule::ChoiceTooManyOptions,
            "256 options",
        ),
    ];
    for (question, rule, detail) in cases {
        let report = check_value(&with_question(&question));

        assert!(!report.is_valid());
        let finding = only(&report, rule);
        assert_eq!(finding.severity, Severity::Error);
        assert_eq!(finding.question.as_deref(), Some("q"));
        assert_eq!(finding.path, "/questions/q/criteria");
        assert!(finding.message.contains(detail), "{}", finding.message);
        assert!(finding.suggestion.is_some());
    }
}

#[test]
fn values_at_the_limits_pass() {
    for question in [
        choice_with_options(255),
        score_with_levels(2),
        score_with_levels(10),
    ] {
        let report = check_value(&with_question(&question));

        assert_eq!(report.findings, [], "{question}");
    }
}

#[test]
fn an_empty_questions_map_and_a_duplicate_question_id_are_rejected() {
    let empty = check_text(r#"{"state": "s", "model": "m", "questions": {}}"#);
    assert_eq!(only(&empty, Rule::QuestionsEmpty).path, "/questions");
    assert!(!empty.is_valid());

    let duplicate = check_text(
        r#"{"state": "s", "model": "m", "questions": {
            "urgent": {"type": "noul", "instructions": "Is it urgent?"},
            "urgent": {"type": "noul", "instructions": "Is it very urgent?"}
        }}"#,
    );
    let finding = only(&duplicate, Rule::DuplicateQuestionId);
    assert_eq!(finding.severity, Severity::Error);
    assert_eq!(finding.question.as_deref(), Some("urgent"));
    assert_eq!(finding.path, "/questions/urgent");
    assert!(!duplicate.is_valid());
}

#[test]
fn a_choice_without_a_way_out_is_a_warning_and_an_error_in_strict_mode() {
    let request = with_question(&json!({
        "type": "choice",
        "instructions": "Which team?",
        "criteria": { "billing": "Payments", "technical": "Bugs", "sales": "Pricing" }
    }));
    let document = Document::from_value(request);

    let relaxed = validate::check_document(&document, &Options::default());
    let finding = only(&relaxed, Rule::ChoiceNoEscapeOption);
    assert_eq!(finding.severity, Severity::Warning);
    assert!(
        relaxed.is_valid(),
        "a warning does not make a request invalid"
    );
    assert_eq!(relaxed.warnings().count(), 1);

    let strict = validate::check_document(&document, &Options::default().strict(true));
    assert_eq!(
        only(&strict, Rule::ChoiceNoEscapeOption).severity,
        Severity::Error
    );
    assert!(!strict.is_valid());

    for way_out in ["other", "None of the above", "not_stated", "N/A", "unknown"] {
        let mut with_exit = document.value().clone();
        with_exit["questions"]["q"]["criteria"][way_out] = Value::Null;
        assert_eq!(check_value(&with_exit).findings, [], "{way_out}");
    }
}

#[test]
fn an_oversized_state_is_an_error_stating_the_estimate_and_the_budget_and_can_be_skipped() {
    let line = "The customer reported that payouts have been failing since Monday. ";
    let oversized = with_question(&json!({ "type": "noul", "instructions": "Is it urgent?" }));
    let mut oversized = oversized;
    oversized["state"] = json!(line.repeat(8000));
    let document = Document::from_value(oversized);

    let report = validate::check_document(&document, &Options::default());
    let size = report.size.clone().unwrap();
    assert!(size.total_tokens > TOTAL_BUDGET_TOKENS);
    for (rule, tokens, budget) in [
        (Rule::SizeTotal, size.total_tokens, TOTAL_BUDGET_TOKENS),
        (
            Rule::SizeQuestion,
            size.largest_question_tokens,
            QUESTION_BUDGET_TOKENS,
        ),
    ] {
        let finding = only(&report, rule);
        assert_eq!(finding.severity, Severity::Error);
        assert!(
            finding.message.contains(&tokens.to_string()),
            "states the estimate: {}",
            finding.message
        );
        assert!(
            finding.message.contains(&budget.to_string()),
            "states the budget: {}",
            finding.message
        );
        assert!(
            finding.message.contains("estimate"),
            "says it is an estimate: {}",
            finding.message
        );
    }
    assert_eq!(
        only(&report, Rule::SizeQuestion).question.as_deref(),
        Some("q")
    );

    let skipped = validate::check_document(&document, &Options::default().skip_size_check(true));
    assert_eq!(skipped.findings, []);
    assert_eq!(skipped.size, None);
}

#[test]
fn a_request_near_a_budget_is_warned_about_not_refused() {
    let line = "The customer reported that payouts have been failing since Monday. ";
    let tokens_per_line = estimate_value_tokens(&json!(line));
    let mut request = with_question(&json!({ "type": "noul", "instructions": "Is it urgent?" }));

    for (fraction, expected) in [
        (0.80, None),
        (0.95, Some("close to the limit")),
        (1.10, Some("may be too large")),
    ] {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let lines = (QUESTION_BUDGET_TOKENS as f64 * fraction / tokens_per_line as f64) as usize;
        request["state"] = json!(line.repeat(lines));

        let report = check_value(&request);

        assert!(report.is_valid(), "{fraction}: {:#?}", report.findings);
        match expected {
            None => assert_eq!(report.findings, [], "{fraction}"),
            Some(wording) => {
                let finding = only(&report, Rule::SizeQuestion);
                assert_eq!(finding.severity, Severity::Warning);
                assert!(finding.message.contains(wording), "{}", finding.message);
            }
        }
    }
}

/// One malformed request per rule, with the path the finding must point at.
fn rule_cases() -> Vec<(Rule, Severity, &'static str, String)> {
    let question = |question: Value| with_question(&question).to_string();
    let noul = json!({ "type": "noul", "instructions": "?" });
    let long_state = json!({ "items": (0..30).collect::<Vec<u32>>() });
    let huge = "word ".repeat(80_000);
    vec![
        (Rule::RequestShape, Severity::Error, "", "[1, 2]".into()),
        (Rule::UnknownField, Severity::Error, "/temperature", json!({ "state": "s", "model": "m", "questions": { "q": noul }, "temperature": 0 }).to_string()),
        (Rule::UnknownField, Severity::Warning, "/questions/q/critera", question(json!({ "type": "noul", "instructions": "?", "critera": {} }))),
        (Rule::StateMissing, Severity::Error, "", json!({ "model": "m", "questions": { "q": noul } }).to_string()),
        (Rule::StateType, Severity::Error, "/state", json!({ "state": 42, "model": "m", "questions": { "q": noul } }).to_string()),
        (Rule::StateType, Severity::Error, "/state", json!({ "state": null, "model": "m", "questions": { "q": noul } }).to_string()),
        (Rule::ModelMissing, Severity::Error, "", json!({ "state": "s", "questions": { "q": noul } }).to_string()),
        (Rule::ModelType, Severity::Error, "/model", json!({ "state": "s", "model": " ", "questions": { "q": noul } }).to_string()),
        (Rule::QuestionsMissing, Severity::Error, "", json!({ "state": "s", "model": "m" }).to_string()),
        (Rule::QuestionsType, Severity::Error, "/questions", json!({ "state": "s", "model": "m", "questions": [noul] }).to_string()),
        (Rule::QuestionsEmpty, Severity::Error, "/questions", json!({ "state": "s", "model": "m", "questions": {} }).to_string()),
        (Rule::QuestionIdEmpty, Severity::Error, "/questions/", json!({ "state": "s", "model": "m", "questions": { "": noul } }).to_string()),
        (Rule::DuplicateQuestionId, Severity::Error, "/questions/q", r#"{"state":"s","model":"m","questions":{"q":{"type":"noul","instructions":"?"},"q":{"type":"noul","instructions":"?"}}}"#.into()),
        (Rule::QuestionShape, Severity::Error, "/questions/q", question(json!("Is it urgent?"))),
        (Rule::TypeMissing, Severity::Error, "/questions/q", question(json!({ "instructions": "?" }))),
        (Rule::TypeUnknown, Severity::Error, "/questions/q/type", question(json!({ "type": "rank", "instructions": "?" }))),
        (Rule::InstructionsMissing, Severity::Error, "/questions/q/instructions", question(json!({ "type": "noul" }))),
        (Rule::InstructionsMissing, Severity::Error, "/questions/q/instructions", question(json!({ "type": "noul", "instructions": "  " }))),
        (Rule::InstructionsType, Severity::Error, "/questions/q/instructions", question(json!({ "type": "noul", "instructions": 7 }))),
        (Rule::CriteriaMissing, Severity::Error, "/questions/q/criteria", question(json!({ "type": "choice", "instructions": "?" }))),
        (Rule::CriteriaMissing, Severity::Error, "/questions/q/criteria", question(json!({ "type": "score", "instructions": "?" }))),
        (Rule::CriteriaType, Severity::Error, "/questions/q/criteria", question(json!({ "type": "score", "instructions": "?", "criteria": { "low": "a" } }))),
        (Rule::CriteriaType, Severity::Error, "/questions/q/criteria", question(json!({ "type": "choice", "instructions": "?", "criteria": ["a", "b"] }))),
        (Rule::CriteriaType, Severity::Error, "/questions/q/criteria", question(json!({ "type": "noul", "instructions": "?", "criteria": "yes" }))),
        (Rule::CriteriaValueType, Severity::Error, "/questions/q/criteria/a", question(json!({ "type": "choice", "instructions": "?", "criteria": { "a": 1, "other": null } }))),
        (Rule::CriteriaValueType, Severity::Error, "/questions/q/criteria/1", question(json!({ "type": "score", "instructions": "?", "criteria": ["low", null] }))),
        (Rule::NoulCriteriaKey, Severity::Error, "/questions/q/criteria/maybe", question(json!({ "type": "noul", "instructions": "?", "criteria": { "true": "y", "maybe": "m" } }))),
        (Rule::ChoiceNoOptions, Severity::Error, "/questions/q/criteria", question(json!({ "type": "choice", "instructions": "?", "criteria": {} }))),
        (Rule::ChoiceTooManyOptions, Severity::Error, "/questions/q/criteria", question(choice_with_options(256))),
        (Rule::ChoiceOptionEmpty, Severity::Error, "/questions/q/criteria/", question(json!({ "type": "choice", "instructions": "?", "criteria": { "": "x", "other": null } }))),
        (Rule::DuplicateOption, Severity::Error, "/questions/q/criteria/a", r#"{"state":"s","model":"m","questions":{"q":{"type":"choice","instructions":"?","criteria":{"a":"x","other":null,"a":"y"}}}}"#.into()),
        (Rule::ScoreTooFewLevels, Severity::Error, "/questions/q/criteria", question(score_with_levels(1))),
        (Rule::ScoreTooManyLevels, Severity::Error, "/questions/q/criteria", question(score_with_levels(11))),
        (Rule::SizeTotal, Severity::Error, "", json!({ "state": huge, "model": "m", "questions": { "q": noul } }).to_string()),
        (Rule::SizeQuestion, Severity::Error, "/state", json!({ "state": huge, "model": "m", "questions": { "q": noul } }).to_string()),
        (Rule::ChoiceNoEscapeOption, Severity::Warning, "/questions/q/criteria", question(json!({ "type": "choice", "instructions": "?", "criteria": { "a": "x", "b": "y", "c": "z" } }))),
        (Rule::ChoiceYesNo, Severity::Warning, "/questions/q/criteria", question(json!({ "type": "choice", "instructions": "?", "criteria": { "Yes": null, "No": null } }))),
        (Rule::PathIndexLongArray, Severity::Warning, "/questions/q/instructions", json!({ "state": long_state, "model": "m", "questions": { "q": { "type": "noul", "instructions": "Is `items[25]` even?" } } }).to_string()),
        (Rule::DuplicateDescription, Severity::Warning, "/questions/q/criteria/b", question(json!({ "type": "choice", "instructions": "?", "criteria": { "a": "Same text", "b": " same TEXT ", "other": null } }))),
        (Rule::DuplicateDescription, Severity::Warning, "/questions/q/criteria/2", question(json!({ "type": "score", "instructions": "?", "criteria": ["Low", "High", "Low"] }))),
        (Rule::DuplicateKey, Severity::Warning, "/state/plan", r#"{"state":{"plan":"free","plan":"pro"},"model":"m","questions":{"q":{"type":"noul","instructions":"?"}}}"#.into()),
    ]
}

#[test]
fn every_rule_fires_where_it_should_with_a_message_and_a_fix() {
    for (rule, severity, path, text) in rule_cases() {
        let report = check_text(&text);

        let finding = report
            .findings
            .iter()
            .find(|finding| finding.rule == rule && finding.path == path)
            .unwrap_or_else(|| panic!("no `{}` at {path:?} in {:#?}", rule.id(), report.findings));
        assert_eq!(finding.severity, severity, "{}", rule.id());
        assert!(!finding.message.is_empty());
        assert!(
            finding
                .suggestion
                .as_deref()
                .is_some_and(|fix| !fix.is_empty()),
            "{} suggests a fix",
            rule.id()
        );
        let in_question = path.starts_with("/questions/") || rule == Rule::SizeQuestion;
        assert_eq!(
            finding.question.is_some(),
            in_question,
            "{} names its question: {finding:?}",
            rule.id()
        );
        assert_eq!(report.is_valid(), report.errors().next().is_none());
    }
}

#[test]
fn no_rule_is_left_without_a_test_case() {
    let covered: HashSet<Rule> = rule_cases().into_iter().map(|(rule, ..)| rule).collect();
    let missing: Vec<&str> = Rule::ALL
        .iter()
        .filter(|rule| !covered.contains(rule))
        .map(|rule| rule.id())
        .collect();

    assert!(missing.is_empty(), "rules without a test case: {missing:?}");
}

#[test]
fn every_problem_is_reported_at_once() {
    let report = check_text(
        r#"{"state": 1, "questions": {
            "a": {"type": "score", "instructions": "?", "criteria": ["only"]},
            "b": {"type": "choise", "instructions": "?"},
            "c": {"type": "noul"},
            "d": {"type": "choice", "instructions": "?", "criteria": {"yes": null, "no": null}}
        }}"#,
    );

    let rules: Vec<Rule> = report.findings.iter().map(|finding| finding.rule).collect();
    assert_eq!(
        rules,
        [
            Rule::StateType,
            Rule::ModelMissing,
            Rule::ScoreTooFewLevels,
            Rule::TypeUnknown,
            Rule::InstructionsMissing,
            Rule::ChoiceNoEscapeOption,
            Rule::ChoiceYesNo,
        ],
        "lints still run when `state` is unusable"
    );
    assert_eq!(
        only(&report, Rule::TypeUnknown).suggestion.as_deref(),
        Some("did you mean `choice`?")
    );
    assert_eq!(report.size, None, "no estimate without a usable state");
}

#[test]
fn a_misspelt_field_gets_a_did_you_mean() {
    let report = check_value(&with_question(
        &json!({ "type": "choice", "instruction": "?", "critera": { "a": null } }),
    ));

    let suggestions: Vec<Option<&str>> = report
        .findings
        .iter()
        .filter(|finding| finding.rule == Rule::UnknownField)
        .map(|finding| finding.suggestion.as_deref())
        .collect();
    assert_eq!(
        suggestions,
        [
            Some("did you mean `instructions`?"),
            Some("did you mean `criteria`?")
        ]
    );
}

#[test]
fn the_model_can_be_left_to_the_caller() {
    let document = Document::from_json_str(
        r#"{"state": "s", "questions": {"q": {"type": "noul", "instructions": "?"}}}"#,
    )
    .unwrap();

    let optional = validate::check_document(&document, &Options::default().model_optional(true));
    assert_eq!(optional.findings, []);

    let wrong = Document::from_json_str(
        r#"{"state": "s", "model": 3, "questions": {"q": {"type": "noul", "instructions": "?"}}}"#,
    )
    .unwrap();
    let report = validate::check_document(&wrong, &Options::default().model_optional(true));
    assert_eq!(only(&report, Rule::ModelType).path, "/model");
}

#[test]
fn short_arrays_and_unrelated_backticks_are_not_linted() {
    let request = json!({
        "state": { "items": [1, 2, 3], "log": (0..30).collect::<Vec<u32>>() },
        "model": "m",
        "questions": { "q": { "type": "noul", "instructions": "Is `items[2]` bigger than `log`? Run `cargo test[0]`." } }
    });

    assert_eq!(check_value(&request).findings, []);
}

#[test]
fn a_report_serialises_for_machine_readers() {
    let report = check_value(&with_question(&score_with_levels(1)));

    let json = serde_json::to_value(&report).unwrap();

    assert_eq!(json["findings"][0]["rule"], "score-too-few-levels");
    assert_eq!(json["findings"][0]["severity"], "error");
    assert_eq!(json["findings"][0]["question"], "q");
    assert_eq!(json["size"]["total_budget_tokens"], 64_000);
}

#[test]
fn the_size_estimate_tracks_real_token_counts() {
    let calibration = &common::load("", "token-calibration")[0].json;
    let baseline = calibration["baseline_input_tokens"].as_u64().unwrap();
    let samples = calibration["samples"].as_array().unwrap();
    assert!(
        samples.len() >= 10,
        "the calibration set covers many kinds of content"
    );

    let (mut total_real, mut total_estimated) = (0_u64, 0_u64);
    for sample in samples {
        let name = sample["name"].as_str().unwrap();
        let real = sample["input_tokens"].as_u64().unwrap() - baseline;
        let estimated = estimate_value_tokens(&sample["state"]);

        let error = percent_error(estimated, real);
        assert!(
            error.abs() <= 15,
            "`{name}`: estimated {estimated}, real {real} ({error:+}%)"
        );
        total_real += real;
        total_estimated += estimated;
    }
    let overall = percent_error(total_estimated, total_real);
    assert!(
        overall.abs() <= 5,
        "overall: estimated {total_estimated}, real {total_real} ({overall:+}%)"
    );
}

fn percent_error(estimated: u64, real: u64) -> i64 {
    let (estimated, real) = (
        i64::try_from(estimated).unwrap(),
        i64::try_from(real).unwrap(),
    );
    (estimated - real) * 100 / real
}
