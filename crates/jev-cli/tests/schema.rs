//! `jev schema` from the outside, and the schemas held against what `jev` really reads and writes.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use assert_cmd::Command;
use jsonschema::Validator;
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SENTINEL_KEY: &str = "sentinel-key-do-not-leak-3c9d";

/// The schemas `jev schema` prints and `schemas/` publishes.
const PUBLISHED: [&str; 5] = ["request", "questions", "batch-record", "output", "error"];

/// `jev`, isolated from the environment of whoever runs the tests, pointed at `server`.
fn jev(server: Option<&MockServer>) -> Command {
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
        "TYPESAFE_BASE_URL",
        "TYPESAFE_DEFAULT_MODEL",
        "TYPESAFE_LOG_LEVEL",
    ] {
        command.env_remove(variable);
    }
    command.env(
        "JEV_CONFIG_DIR",
        Path::new(env!("CARGO_TARGET_TMPDIR")).join("no-config"),
    );
    command.env("TYPESAFE_API_KEY", SENTINEL_KEY);
    command.env(
        "TYPESAFE_BASE_URL",
        server.map_or_else(|| "http://127.0.0.1:9".to_owned(), MockServer::uri),
    );
    command.timeout(Duration::from_secs(60));
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

async fn run_async(mut command: Command) -> Run {
    tokio::task::spawn_blocking(move || run(&mut command))
        .await
        .unwrap()
}

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("not JSON ({error}): {text:?}"))
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/eval")
        .join(name)
        .to_str()
        .unwrap()
        .to_owned()
}

/// What `jev schema <name>` prints, checked to be a valid draft 2020-12 schema.
fn schema(name: &str) -> Value {
    let run = run(jev(None).args(["schema", name]));
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stderr, "");
    let schema = json_of(&run.stdout);
    assert_eq!(
        schema.get("$schema").and_then(Value::as_str),
        Some("https://json-schema.org/draft/2020-12/schema"),
        "{name}"
    );
    assert!(
        jsonschema::meta::is_valid(&schema),
        "`jev schema {name}` is not a valid JSON Schema"
    );
    schema
}

fn validator(name: &str) -> Validator {
    jsonschema::draft202012::new(&schema(name)).unwrap()
}

fn assert_valid(validator: &Validator, instance: &Value, what: &str) {
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|error| format!("{} at {}", error, error.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{what} does not match: {errors:?}");
}

#[test]
fn every_schema_is_valid_draft_2020_12_and_carries_a_title_and_a_description() {
    for name in PUBLISHED {
        let schema = schema(name);
        assert!(
            schema["title"]
                .as_str()
                .is_some_and(|title| !title.is_empty())
        );
        assert!(
            schema["description"]
                .as_str()
                .is_some_and(|text| text.len() > 50)
        );
        assert!(
            !schema.to_string().contains("\"$ref\""),
            "`{name}` must be self-contained, with its definitions inlined"
        );
    }
}

/// The published schemas in `schemas/` are the snapshots: they must be exactly what this build
/// prints. After an intended change, run `make schemas` and review the diff.
#[test]
fn the_published_schemas_are_what_jev_prints() {
    for name in PUBLISHED {
        let run = run(jev(None).args(["schema", name, "-o", "json"]));
        let file = workspace().join(format!("schemas/{name}.schema.json"));
        let published = std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("{}: {error}; run `make schemas`", file.display()));
        assert!(
            run.stdout == published,
            "{} is out of date; run `make schemas` and review the diff",
            file.display()
        );
    }
}

#[test]
fn the_request_schema_accepts_the_upstream_examples_and_the_repository_s_own_requests() {
    let validator = validator("request");
    let fixtures = workspace().join("crates/jev-client/tests/fixtures");
    let mut checked = 0;
    for directory in ["api-reference", "live"] {
        for entry in std::fs::read_dir(fixtures.join(directory)).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name.starts_with("request-") {
                let request = json_of(&std::fs::read_to_string(&path).unwrap());
                assert_valid(&validator, &request, &name);
                checked += 1;
            }
        }
    }
    assert!(
        checked >= 5,
        "expected the upstream examples, found {checked}"
    );

    let triage = json_of(&std::fs::read_to_string(fixture("triage.json")).unwrap());
    assert_valid(&validator, &triage, "triage.json");
    let triage: Value =
        serde_saphyr::from_str(&std::fs::read_to_string(fixture("triage.yaml")).unwrap()).unwrap();
    assert_valid(&validator, &triage, "triage.yaml");
}

#[test]
fn the_request_schema_rejects_an_eleven_level_score_and_a_file_without_state() {
    let request = validator("request");
    let eleven = json_of(&std::fs::read_to_string(fixture("eleven-levels.json")).unwrap());
    let levels = eleven
        .pointer("/questions")
        .and_then(Value::as_object)
        .and_then(|questions| questions.values().next())
        .and_then(|question| question["criteria"].as_array())
        .map(Vec::len);
    assert_eq!(levels, Some(11), "the fixture is what its name says");
    let mut complete = eleven.clone();
    complete["state"] = json!("I was charged twice.");
    complete["model"] = json!("jev-latest");

    assert!(!request.is_valid(&complete));
    let mut ten = complete.clone();
    for question in ten["questions"].as_object_mut().unwrap().values_mut() {
        question["criteria"].as_array_mut().unwrap().truncate(10);
    }
    assert_valid(&request, &ten, "the same request with ten levels");

    let questions_only: Value =
        serde_saphyr::from_str(&std::fs::read_to_string(fixture("questions.yaml")).unwrap())
            .unwrap();
    assert!(
        !request.is_valid(&questions_only),
        "a request has its state and model"
    );
    assert_valid(&validator("questions"), &questions_only, "questions.yaml");
    assert!(!validator("questions").is_valid(&complete));
}

#[test]
fn the_questions_schema_still_needs_questions_and_rejects_unknown_fields() {
    let validator = validator("questions");
    assert!(!validator.is_valid(&json!({ "state": "s", "model": "m" })));
    assert!(!validator.is_valid(&json!({
        "questions": { "q": { "type": "noul", "instructions": "?" } },
        "temperature": 0
    })));
    assert!(!validator.is_valid(&json!({
        "questions": { "q": { "type": "noul", "instructions": "?", "critera": {} } }
    })));
}

async fn api(body: &Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-typesafe-request-id", "req_schema")
                .set_body_json(body),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test(flavor = "multi_thread")]
async fn real_output_of_eval_and_of_every_shortcut_matches_the_output_schema() {
    let validator = validator("output");
    let response = json_of(&std::fs::read_to_string(fixture("response.json")).unwrap());

    let server = api(&response).await;
    let mut eval = jev(Some(&server));
    eval.args(["eval", "-f", &fixture("triage.json"), "-o", "json"]);
    let mut gated = jev(Some(&server));
    gated.args([
        "eval",
        "-f",
        &fixture("triage.json"),
        "--assert",
        "is_urgent >= 0.5",
    ]);
    // A model with no known price and no request id: `cost_usd` and `request_id` are null.
    let unpriced = MockServer::start().await;
    let mut body = response.clone();
    body["model"] = json!("jev-9.0.0");
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&unpriced)
        .await;
    let mut nulls = jev(Some(&unpriced));
    nulls.args(["eval", "-f", &fixture("triage.json")]);

    for command in [eval, gated, nulls] {
        let run = run_async(command).await;
        assert_eq!(run.code, 0, "{}", run.stderr);
        assert_valid(&validator, &json_of(&run.stdout), "`jev eval` output");
    }

    let shortcuts = [
        (
            json!({ "type": "noul", "noul": 0.3 }),
            vec!["noul", "Is it urgent?", "--fail-under", "0.7"],
            10,
        ),
        (
            json!({ "type": "choice", "choice": "billing", "confidence": 0.9, "probabilities": { "billing": 0.95, "other": 0.05 } }),
            vec![
                "choice",
                "Which team?",
                "--option",
                "billing",
                "--option",
                "other",
            ],
            0,
        ),
        (
            json!({ "type": "score", "score": 0.4, "confidence": 0.7, "legend": { "0": "Calm", "1": "Angry" }, "probabilities": { "0": 0.6, "1": 0.4 } }),
            vec!["score", "How angry?", "--level", "Calm", "--level", "Angry"],
            0,
        ),
    ];
    for (answer, arguments, exit) in shortcuts {
        let server = api(&json!({
            "model": "jev-1.13.0",
            "answers": { "answer": answer },
            "usage": { "input_tokens": 120, "output_tokens": 10 }
        }))
        .await;
        let mut command = jev(Some(&server));
        command
            .args(&arguments)
            .args(["--state", "Payouts are failing!"]);
        let run = run_async(command).await;
        assert_eq!(run.code, exit, "{arguments:?}: {}", run.stderr);
        assert_valid(
            &validator,
            &json_of(&run.stdout),
            &format!("`jev {}` output", arguments[0]),
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn real_errors_match_the_error_schema_and_never_carry_the_key() {
    let validator = validator("error");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "detail": { "error_type": "authentication_error", "message": "Invalid API key." }
        })))
        .mount(&server)
        .await;
    let mut rejected = jev(Some(&server));
    rejected.args(["eval", "-f", &fixture("triage.json"), "-o", "json"]);
    let mut invalid = jev(None);
    invalid.args([
        "eval",
        "-f",
        &fixture("eleven-levels.json"),
        "--state",
        "s",
        "-o",
        "json",
    ]);
    let mut usage = jev(None);
    usage.args(["eval", "-f", "does-not-exist.json", "-o", "json"]);

    for (command, exit) in [(rejected, 3), (invalid, 2), (usage, 2)] {
        let run = run_async(command).await;
        assert_eq!(run.code, exit, "{}", run.stderr);
        assert_eq!(run.stdout, "");
        assert!(!run.stderr.contains(SENTINEL_KEY));
        let error = json_of(&run.stderr);
        assert_valid(&validator, &error, "the error object");
        assert_eq!(error["error"]["exit_code"], exit);
    }

    assert!(
        !validator.is_valid(&json!({ "error": { "code": "usage", "message": "m" } })),
        "every field is always present, so the schema requires every field"
    );
}

#[test]
fn a_schema_prints_as_yaml_when_asked() {
    let run = run(jev(None).args(["schema", "error", "-o", "yaml"]));

    assert_eq!(run.code, 0);
    assert_eq!(run.stderr, "");
    let yaml: Value = serde_saphyr::from_str(&run.stdout).unwrap();
    assert_eq!(yaml, schema("error"));
}

#[test]
fn schema_needs_no_key_and_no_configuration() {
    let mut command = jev(None);
    command
        .env_remove("TYPESAFE_API_KEY")
        .env(
            "JEV_CONFIG_DIR",
            Path::new(env!("CARGO_TARGET_TMPDIR")).join("schema-no-such-dir"),
        )
        .args(["schema", "request", "--field", "title"]);

    let run = run(&mut command);

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "jev request\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn real_batch_records_of_both_kinds_match_the_batch_record_schema() {
    let validator = validator("batch-record");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(body_string_contains("FAIL"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "detail": { "error_type": "invalid_request_error", "message": "Not acceptable." }
        })))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-typesafe-request-id", "req_row")
                .set_body_json(json!({
                    "model": "jev-1.13.0",
                    "answers": { "is_urgent": { "type": "noul", "noul": 0.9 } },
                    "usage": { "input_tokens": 300, "output_tokens": 20 }
                })),
        )
        .mount(&server)
        .await;
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("schema-batch");
    std::fs::create_dir_all(&dir).unwrap();
    let rows = dir.join("rows.jsonl");
    std::fs::write(
        &rows,
        "{\"ticket\": \"a\", \"text\": \"fine\"}\n{\"ticket\": 7, \"text\": \"FAIL\"}\n",
    )
    .unwrap();
    let mut command = jev(Some(&server));
    command
        .args(["batch", "run", "-f", &fixture("questions.yaml"), "--input"])
        .arg(&rows)
        .args(["--state-field", "text", "--id-field", "ticket", "-q"]);

    let run = run_async(command).await;

    assert_eq!(run.code, 7, "{}", run.stderr);
    let records: Vec<Value> = run.stdout.lines().map(json_of).collect();
    let statuses: Vec<&str> = records
        .iter()
        .filter_map(|record| record.get("status").and_then(Value::as_str))
        .collect();
    assert_eq!(statuses.len(), 2);
    assert!(statuses.contains(&"ok") && statuses.contains(&"error"));
    for record in &records {
        assert_valid(&validator, record, "a batch record");
        assert!(!record.to_string().contains(SENTINEL_KEY));
    }
    assert!(
        !validator.is_valid(&json!({ "id": 1, "status": "ok", "model": "m" })),
        "an ok record has every field"
    );
}
