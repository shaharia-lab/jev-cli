//! `jev validate` from the outside.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::Path;

use assert_cmd::Command;
use serde_json::{Value, json};

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/validate")
        .join(name)
        .to_str()
        .unwrap()
        .to_owned()
}

/// `jev` with no API key, and a base URL nothing listens on: validation needs neither.
fn jev() -> Command {
    let mut command = Command::cargo_bin("jev").unwrap();
    for variable in [
        "CI",
        "NO_COLOR",
        "TERM",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "JEV_OUTPUT",
        "JEV_PROFILE",
        "JEV_NO_INPUT",
        "TYPESAFE_API_KEY",
        "TYPESAFE_DEFAULT_MODEL",
        "TYPESAFE_LOG_LEVEL",
    ] {
        command.env_remove(variable);
    }
    command.env(
        "JEV_CONFIG_DIR",
        Path::new(env!("CARGO_TARGET_TMPDIR")).join("no-config"),
    );
    // Never the real keychain of whoever runs the tests.
    command.env("JEV_NO_KEYCHAIN", "1");
    command.env("TYPESAFE_BASE_URL", "http://127.0.0.1:9");
    command
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(command: &mut Command) -> Run {
    let output = command.output().unwrap();
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("not JSON ({error}): {text:?}"))
}

#[test]
fn an_eleven_level_score_exits_2_naming_the_question_the_path_and_the_rule() {
    let run = run(jev().args(["validate", "-f", &fixture("eleven-levels.yaml")]));

    assert_eq!(run.code, 2);
    let report = json_of(&run.stdout);
    assert_eq!(report["valid"], false);
    assert_eq!(report["summary"], json!({ "errors": 1, "warnings": 0 }));
    let finding = &report["findings"][0];
    assert_eq!(finding["question"], "rating");
    assert_eq!(finding["path"], "/questions/rating/criteria");
    assert_eq!(finding["rule"], "score-too-many-levels");
    assert!(
        finding["message"]
            .as_str()
            .unwrap()
            .contains("11 levels; the limit is 10"),
        "{finding}"
    );

    let error = &json_of(&run.stderr)["error"];
    assert_eq!(error["exit_code"], 2);
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .ends_with("is not valid: 1 problem found"),
        "{error}"
    );
}

#[test]
fn the_json_report_has_a_stable_documented_shape() {
    let run = run(jev().args([
        "validate",
        "-f",
        &fixture("eleven-levels.yaml"),
        "-o",
        "json",
    ]));

    let expected = json!({
        "file": fixture("eleven-levels.yaml"),
        "valid": false,
        "strict": false,
        "summary": { "errors": 1, "warnings": 0 },
        "findings": [{
            "severity": "error",
            "rule": "score-too-many-levels",
            "question": "rating",
            "path": "/questions/rating/criteria",
            "message": "score `rating` has 11 levels; the limit is 10 (the API fails with a server error beyond it)",
            "suggestion": "merge neighbouring levels; 3 to 5 well-separated levels usually work best"
        }],
        "size": {
            "state_tokens": 6,
            "questions_tokens": 85,
            "overhead_tokens": 270,
            "total_tokens": 361,
            "total_budget_tokens": 64000,
            "largest_question_tokens": 361,
            "largest_question": "rating",
            "question_budget_tokens": 32000
        }
    });
    assert_eq!(json_of(&run.stdout), expected);
    let keys: Vec<&str> = expected
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let actual = json_of(&run.stdout);
    assert_eq!(
        actual
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        keys,
        "key order is part of the shape"
    );
}

#[test]
fn lint_warnings_pass_unless_strict() {
    let relaxed = run(jev().args(["validate", "-f", &fixture("lint-only.yaml")]));
    let strict = run(jev().args(["validate", "-f", &fixture("lint-only.yaml"), "--strict"]));

    assert_eq!((relaxed.code, relaxed.stderr.as_str()), (0, ""));
    let report = json_of(&relaxed.stdout);
    assert_eq!(
        (
            report["valid"].as_bool(),
            report["summary"]["warnings"].as_u64()
        ),
        (Some(true), Some(1))
    );
    assert_eq!(report["findings"][0]["rule"], "choice-no-escape-option");
    assert_eq!(report["findings"][0]["severity"], "warning");

    assert_eq!(strict.code, 2);
    let report = json_of(&strict.stdout);
    assert_eq!(
        (report["valid"].as_bool(), report["strict"].as_bool()),
        (Some(false), Some(true))
    );
    assert_eq!(report["findings"][0]["severity"], "error");
    assert!(
        json_of(&strict.stderr)["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("--strict counts warnings as errors")
    );
}

#[test]
fn it_needs_no_key_no_network_and_no_state() {
    // No TYPESAFE_API_KEY, a base URL nothing listens on, and a questions-only file with stdin
    // left open: none of that may matter.
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("jev"))
        .args(["validate", "-f", &fixture("questions-only.yaml")])
        .env(
            "JEV_CONFIG_DIR",
            Path::new(env!("CARGO_TARGET_TMPDIR")).join("no-config"),
        )
        .env("TYPESAFE_BASE_URL", "http://127.0.0.1:9")
        .env_remove("TYPESAFE_API_KEY")
        .env_remove("JEV_PROFILE")
        .env_remove("JEV_OUTPUT")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let held_open = child.stdin.take().unwrap();
    let output = child.wait_with_output().unwrap();
    drop(held_open);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json_of(&String::from_utf8(output.stdout).unwrap());
    assert_eq!(report["valid"], true);
    assert_eq!(report["findings"], json!([]));
    assert_eq!(
        report["size"],
        Value::Null,
        "without a state there is nothing to measure"
    );
}

#[test]
fn a_state_can_be_supplied_so_that_the_size_is_checked() {
    let huge = "word ".repeat(80_000);
    let sized = run(jev().args([
        "validate",
        "-f",
        &fixture("questions-only.yaml"),
        "--state",
        "short",
    ]));
    let too_big = run(jev()
        .args([
            "validate",
            "-f",
            &fixture("questions-only.yaml"),
            "--state-file",
            "-",
        ])
        .write_stdin(huge.clone()));
    let skipped = run(jev()
        .args([
            "validate",
            "-f",
            &fixture("questions-only.yaml"),
            "--state-file",
            "-",
            "--skip-size-check",
        ])
        .write_stdin(huge));

    assert_eq!(sized.code, 0);
    assert!(
        json_of(&sized.stdout)["size"]["total_tokens"]
            .as_u64()
            .unwrap()
            > 270
    );
    assert_eq!(too_big.code, 2);
    let rules: Vec<String> = json_of(&too_big.stdout)["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["rule"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(rules, ["size-total", "size-question"]);
    assert_eq!(skipped.code, 0);
}

#[test]
fn a_person_sees_findings_grouped_by_question_with_the_fix() {
    let request = json!({
        "state": 7,
        "questions": {
            "team": { "type": "choice", "instructions": "Which?", "criteria": { "yes": null, "no": null } },
            "rating": { "type": "score", "instructions": "How bad?", "criteria": ["only"] }
        }
    });

    let run = run(jev()
        .args(["validate", "-f", "-", "-o", "table"])
        .write_stdin(request.to_string()));

    let expected = "\
stdin: invalid, 2 errors, 2 warnings

request
  error    state-type  /state
           `state` must be a string, an object or an array, not a number
           fix: wrap a single value in a string or an object

question `rating`
  error    score-too-few-levels  /questions/rating/criteria
           score `rating` has 1 level(s); at least 2 are needed (the API accepts fewer and returns a meaningless answer)
           fix: describe both ends of the scale, or use a noul for a yes/no question

question `team`
  warning  choice-no-escape-option  /questions/team/criteria
           choice `team` has no way out; the model must pick one of its options even when none fits
           fix: add an option such as `other`, `none_of_the_above` or `not_stated`
  warning  choice-yes-no  /questions/team/criteria
           choice `team` is a yes/no question
           fix: use a noul: it returns the probability of yes, which a threshold can be tuned against
";
    assert_eq!(run.stdout, expected);
    assert_eq!(run.code, 2);
    assert_eq!(
        run.stderr,
        "error: stdin is not valid: 2 problems found\n  hint: fix the findings above\n"
    );
}

#[test]
fn a_valid_file_says_so_and_jsonl_is_one_finding_per_line() {
    let valid = run(jev().args([
        "validate",
        "-f",
        &fixture("questions-only.yaml"),
        "-o",
        "table",
    ]));
    let lines = run(jev().args(["validate", "-f", &fixture("lint-only.yaml"), "-o", "jsonl"]));

    assert_eq!(
        (valid.code, valid.stdout.as_str()),
        (
            0,
            format!("{}: valid\n", fixture("questions-only.yaml")).as_str()
        )
    );
    assert_eq!(lines.stdout.lines().count(), 1);
    assert_eq!(json_of(&lines.stdout)["rule"], "choice-no-escape-option");
}

#[test]
fn a_file_that_is_not_a_request_at_all_is_reported_not_crashed_on() {
    let not_an_object = run(jev().args(["validate", "-f", "-"]).write_stdin("[1, 2, 3]"));
    let not_parseable = run(jev()
        .args(["validate", "-f", "-"])
        .write_stdin("{\"questions\": "));
    let missing = run(jev().args(["validate", "-f", "/nonexistent/request.yaml"]));

    assert_eq!(not_an_object.code, 2);
    assert_eq!(
        json_of(&not_an_object.stdout)["findings"][0]["rule"],
        "request-shape"
    );
    for failed in [not_parseable, missing] {
        assert_eq!((failed.code, failed.stdout.as_str()), (2, ""));
        assert_eq!(json_of(&failed.stderr)["error"]["code"], "usage");
    }
}
