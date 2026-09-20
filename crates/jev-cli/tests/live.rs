//! The live smoke suite (PRD TEST-3): a handful of calls to the real TypeSafe API, to notice when
//! the server changes under us.
//!
//! Every test here costs money, so all of them are ignored by default and none runs in `make check`
//! or in CI on a pull request. Run them on purpose, one at a time so a shared key is not
//! rate-limited:
//!
//! ```text
//! TYPESAFE_API_KEY=... cargo test -p jev-cli --test live -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Without `TYPESAFE_API_KEY` each test says it was skipped and passes, unless
//! `JEV_LIVE_SMOKE_REQUIRED` is set, as the nightly workflow (`.github/workflows/live-smoke.yml`)
//! does so that a missing secret fails loudly instead of passing quietly.
//!
//! Two kinds of test:
//!
//! - `jev` itself, built from this tree, for one call of each question type through `jev eval` and
//!   through each shortcut, and `jev models list`. These check the output contract end to end.
//! - `jev-client`'s transport directly, for the server quirks that `jev` works around
//!   (`docs/api-behaviour.md`). `jev` validates before it sends, so it cannot send
//!   these requests; the transport sends a request as written. When one of these fails, the server
//!   changed: update the finding, and the validation rule that depends on it.
//!
//! Answers are not bit-for-bit deterministic, so the assertions are about shapes and ranges, never
//! exact probabilities. Each test prints the input tokens it used (`live-smoke tokens: N`), which
//! the workflow adds up; a whole run is a few thousand tokens, a small fraction of a cent.
//!
//! `JEV_LIVE_SMOKE_BREAK_EXPECTATION` makes `jev models list` expect a model that does not exist,
//! so the workflow's failure path (filing the tracking issue) can be exercised on demand.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::Path;
use std::time::Duration;

use assert_cmd::Command;
use jev_client::{ApiKey, ErrorKind, HttpTransport, Request, Transport};
use serde_json::{Value, json};

/// The alias every test asks for. The response names the versioned model behind it.
const ALIAS: &str = "jev-latest";

/// The API key, or `None` (after saying so) when the suite should be skipped.
///
/// The key is never printed, and nothing derived from it is either.
fn key() -> Option<String> {
    match std::env::var("TYPESAFE_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => {
            assert!(
                std::env::var_os("JEV_LIVE_SMOKE_REQUIRED").is_none(),
                "JEV_LIVE_SMOKE_REQUIRED is set, but TYPESAFE_API_KEY is not"
            );
            eprintln!("live-smoke: skipped, TYPESAFE_API_KEY is not set");
            None
        }
    }
}

/// `jev`, isolated from the environment of whoever runs the tests except for the key, talking to
/// the real API.
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
        "TYPESAFE_BASE_URL",
        "TYPESAFE_DEFAULT_MODEL",
        "TYPESAFE_LOG_LEVEL",
    ] {
        command.env_remove(variable);
    }
    command.env(
        "JEV_CONFIG_DIR",
        Path::new(env!("CARGO_TARGET_TMPDIR")).join("live-no-config"),
    );
    command.timeout(Duration::from_secs(120));
    command
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Runs `jev` and checks what every successful call has in common: exit 0, nothing on stderr, and
/// no trace of the key anywhere.
fn run_ok(mut command: Command, key: &str) -> Value {
    let output = command.output().unwrap();
    let run = Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    };
    // Plain `assert!`s: a failure message must never quote the key.
    assert!(!run.stdout.contains(key), "the key appeared on stdout");
    assert!(!run.stderr.contains(key), "the key appeared on stderr");
    assert_eq!(
        (run.code, run.stderr.as_str()),
        (0, ""),
        "stdout: {}",
        run.stdout
    );
    serde_json::from_str(&run.stdout)
        .unwrap_or_else(|error| panic!("not JSON ({error}): {:?}", run.stdout))
}

/// A versioned model id such as `jev-1.13.0`, as opposed to an alias.
fn assert_versioned_model(model: &Value) {
    let model = model.as_str().unwrap_or_default();
    assert!(
        model
            .strip_prefix("jev-")
            .is_some_and(|version| version.starts_with(|c: char| c.is_ascii_digit())),
        "expected a versioned model id, got {model:?}"
    );
}

fn assert_probability(value: &Value, what: &str) {
    let number = value
        .as_f64()
        .unwrap_or_else(|| panic!("{what} is not a number: {value}"));
    assert!(
        (0.0..=1.0).contains(&number),
        "{what} out of range: {number}"
    );
}

/// Probabilities keyed by option or level: every key expected, each in range, summing to about 1.
fn assert_distribution(probabilities: &Value, keys: &[&str], what: &str) {
    let probabilities = probabilities
        .as_object()
        .unwrap_or_else(|| panic!("{what} probabilities are not an object: {probabilities}"));
    let mut found: Vec<&str> = probabilities.keys().map(String::as_str).collect();
    found.sort_unstable();
    let mut expected = keys.to_vec();
    expected.sort_unstable();
    assert_eq!(found, expected, "{what} probability keys");
    for (key, probability) in probabilities {
        assert_probability(probability, &format!("{what} probability of {key}"));
    }
    let sum: f64 = probabilities.values().filter_map(Value::as_f64).sum();
    assert!(
        (sum - 1.0).abs() < 0.02,
        "{what} probabilities sum to {sum}"
    );
}

fn assert_noul(answer: &Value) {
    assert_eq!(answer["type"], "noul", "{answer}");
    assert_probability(&answer["noul"], "noul");
}

fn assert_choice(answer: &Value, options: &[&str]) {
    assert_eq!(answer["type"], "choice", "{answer}");
    let winner = answer["choice"].as_str().unwrap_or_default();
    assert!(
        options.contains(&winner),
        "choice {winner:?} is not an option"
    );
    assert_probability(&answer["confidence"], "choice confidence");
    assert_distribution(&answer["probabilities"], options, "choice");
}

/// A Score over `levels` levels. Over HTTP, `legend` and `probabilities` are keyed by the level
/// number as a string, and `legend` echoes the level descriptions.
fn assert_score(answer: &Value, levels: &[&str]) {
    assert_eq!(answer["type"], "score", "{answer}");
    let top = f64::from(u8::try_from(levels.len() - 1).unwrap());
    let score = answer["score"].as_f64().unwrap_or(-1.0);
    assert!((0.0..=top).contains(&score), "score out of range: {score}");
    assert_probability(&answer["confidence"], "score confidence");
    let numbers: Vec<String> = (0..levels.len()).map(|level| level.to_string()).collect();
    let numbers: Vec<&str> = numbers.iter().map(String::as_str).collect();
    assert_distribution(&answer["probabilities"], &numbers, "score");
    for (number, description) in numbers.iter().zip(levels) {
        let legend = answer.pointer(&format!("/legend/{number}"));
        assert_eq!(legend, Some(&json!(description)), "legend of {number}");
    }
}

/// The parts of the result envelope that are always there, and the tokens it reports.
fn assert_envelope(envelope: &Value, label: &str) {
    assert_versioned_model(&envelope["model"]);
    assert_eq!(envelope["requested_model"], ALIAS);
    let request_id = envelope["request_id"].as_str().unwrap_or_default();
    assert!(!request_id.is_empty(), "no request id: {envelope}");
    assert!(envelope["latency_ms"].is_u64(), "{envelope}");
    let tokens = envelope
        .pointer("/usage/input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    assert!(tokens > 0, "no input tokens: {envelope}");
    assert!(
        envelope
            .pointer("/usage/output_tokens")
            .is_some_and(Value::is_u64),
        "{envelope}"
    );
    // `cost_usd` is `null` for a model whose price `jev` does not know yet; otherwise it is small.
    if let Some(cost) = envelope["cost_usd"].as_f64() {
        assert!(cost > 0.0 && cost < 0.001, "cost {cost}");
    }
    report_tokens(tokens, label);
}

fn report_tokens(tokens: u64, label: &str) {
    eprintln!("live-smoke tokens: {tokens} ({label})");
}

const STATE: &str = "Hi team, the invoice for March was charged twice to our card. Please refund \
                     the duplicate charge; we have been customers for six years.";
const OPTIONS: [&str; 3] = ["billing", "technical", "other"];
const LEVELS: [&str; 3] = ["Calm", "Annoyed", "Furious"];

#[test]
#[ignore = "calls the real TypeSafe API and costs money: see the module documentation"]
fn models_list_names_the_aliases() {
    let Some(key) = key() else { return };
    // The workflow sets it to an empty string when it is not wanted.
    let broken =
        std::env::var_os("JEV_LIVE_SMOKE_BREAK_EXPECTATION").is_some_and(|value| !value.is_empty());
    let expected = if broken {
        "jev-a-model-that-does-not-exist"
    } else {
        ALIAS
    };
    let mut command = jev();
    command.args(["models", "list", "-o", "json"]);
    let listed = run_ok(command, &key);
    let models = listed["models"].as_array().unwrap();
    for model in models {
        assert!(model["name"].is_string(), "{model}");
    }
    assert!(
        models.iter().any(|model| model["name"] == expected),
        "{expected} is not listed: {listed}"
    );
}

#[test]
#[ignore = "calls the real TypeSafe API and costs money: see the module documentation"]
fn eval_answers_one_question_of_each_type() {
    let Some(key) = key() else { return };
    let request = json!({
        "model": ALIAS,
        "state": STATE,
        "questions": {
            "refund": { "type": "noul", "instructions": "Does the customer ask for a refund?" },
            "team": {
                "type": "choice",
                "instructions": "Which team should handle this?",
                "criteria": { "billing": null, "technical": null, "other": "None of the above" }
            },
            "mood": {
                "type": "score",
                "instructions": "How upset is the customer?",
                "criteria": LEVELS
            }
        }
    });
    let mut command = jev();
    command
        .args(["eval", "-f", "-", "-o", "json"])
        .write_stdin(request.to_string());
    let envelope = run_ok(command, &key);
    assert_envelope(&envelope, "eval");
    let answers = envelope["answers"].as_object().unwrap();
    assert_eq!(answers.len(), 3, "{envelope}");
    assert_noul(&answers["refund"]);
    assert_choice(&answers["team"], &OPTIONS);
    assert_score(&answers["mood"], &LEVELS);
}

#[test]
#[ignore = "calls the real TypeSafe API and costs money: see the module documentation"]
fn noul_answers_at_the_top_level() {
    let Some(key) = key() else { return };
    let mut command = jev();
    command.args([
        "noul",
        "Does the customer ask for a refund?",
        "--state",
        STATE,
        "-o",
        "json",
    ]);
    let envelope = run_ok(command, &key);
    assert_envelope(&envelope, "noul");
    assert_noul(&envelope);
}

#[test]
#[ignore = "calls the real TypeSafe API and costs money: see the module documentation"]
fn choice_answers_at_the_top_level() {
    let Some(key) = key() else { return };
    let mut command = jev();
    command.args([
        "choice",
        "Which team should handle this?",
        "--state",
        STATE,
        "-o",
        "json",
    ]);
    for option in OPTIONS {
        command.args(["--option", option]);
    }
    let envelope = run_ok(command, &key);
    assert_envelope(&envelope, "choice");
    assert_choice(&envelope, &OPTIONS);
}

#[test]
#[ignore = "calls the real TypeSafe API and costs money: see the module documentation"]
fn score_answers_at_the_top_level() {
    let Some(key) = key() else { return };
    let mut command = jev();
    command.args([
        "score",
        "How upset is the customer?",
        "--state",
        STATE,
        "-o",
        "json",
    ]);
    for level in LEVELS {
        command.args(["--level", level]);
    }
    let envelope = run_ok(command, &key);
    assert_envelope(&envelope, "score");
    assert_score(&envelope, &LEVELS);
}

// The server quirks, sent as written through `jev-client`'s transport.

fn transport(key: &str) -> HttpTransport {
    HttpTransport::builder(ApiKey::new(key.to_owned()).unwrap())
        .user_agent("jev-live-smoke")
        .build()
        .unwrap()
}

fn request(question: &Value) -> Request {
    serde_json::from_value(json!({
        "model": ALIAS,
        "state": "The parcel arrived on time and in one piece.",
        "questions": { "q": question }
    }))
    .unwrap()
}

/// Sends `request` and returns its one answer as the server wrote it, with the resolved model and
/// request id checked on the way.
async fn answer(transport: &HttpTransport, request: &Request, label: &str) -> Value {
    let reply = transport.evaluate(request).await.unwrap();
    assert!(reply.meta.request_id.is_some(), "no request id");
    assert_versioned_model(&json!(reply.body.model));
    report_tokens(reply.body.usage.input_tokens, label);
    let raw: Value = serde_json::from_str(reply.raw_body.as_deref().unwrap()).unwrap();
    raw.pointer("/answers/q").cloned().unwrap_or(Value::Null)
}

/// Sends `request`, which the server should refuse, and returns the error.
async fn refusal(transport: &HttpTransport, request: &Request) -> jev_client::Error {
    match transport.evaluate(request).await {
        Ok(reply) => panic!(
            "the server now accepts this request (model {}): update the finding",
            reply.body.model
        ),
        Err(error) => error,
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "calls the real TypeSafe API and costs money: see the module documentation"]
async fn the_server_still_answers_a_score_with_one_level() {
    let Some(key) = key() else { return };
    let transport = transport(&key);
    let question = json!({ "type": "score", "instructions": "How positive is this?", "criteria": ["Neutral"] });
    let answer = answer(&transport, &request(&question), "1-level score").await;
    assert_score(&answer, &["Neutral"]);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "calls the real TypeSafe API and costs money: see the module documentation"]
async fn the_server_still_refuses_a_score_with_eleven_levels_with_a_400() {
    let Some(key) = key() else { return };
    let transport = transport(&key);
    let levels: Vec<String> = (0..11).map(|level| format!("L{level}")).collect();
    let question =
        json!({ "type": "score", "instructions": "How positive is this?", "criteria": levels });
    let error = refusal(&transport, &request(&question)).await;
    assert_eq!(
        (error.kind(), error.status()),
        (ErrorKind::BadRequest, Some(400)),
        "{error}"
    );
    assert!(error.message().contains("levels"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "calls the real TypeSafe API and costs money: see the module documentation"]
async fn the_server_still_refuses_a_null_score_level_with_a_422() {
    let Some(key) = key() else { return };
    let transport = transport(&key);
    let question = json!({ "type": "score", "instructions": "How positive is this?", "criteria": ["Low", null] });
    let error = refusal(&transport, &request(&question)).await;
    assert_eq!(
        (error.kind(), error.status()),
        (ErrorKind::Unprocessable, Some(422)),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "calls the real TypeSafe API and costs money: see the module documentation"]
async fn the_server_still_refuses_a_noul_with_neither_criteria_nor_instructions_with_a_400() {
    let Some(key) = key() else { return };
    let transport = transport(&key);
    let error = refusal(&transport, &request(&json!({ "type": "noul" }))).await;
    assert_eq!(
        (error.kind(), error.status()),
        (ErrorKind::BadRequest, Some(400)),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "calls the real TypeSafe API and costs money: see the module documentation"]
async fn the_server_still_refuses_an_unknown_top_level_field_with_an_opaque_400() {
    let Some(key) = key() else { return };
    let transport = transport(&key);
    let question = json!({ "type": "noul", "instructions": "Did it arrive?" });
    let mut request: Value = serde_json::to_value(request(&question)).unwrap();
    request["temperature"] = json!(0.5);
    let request: Request = serde_json::from_value(request).unwrap();
    let error = refusal(&transport, &request).await;
    assert_eq!(
        (error.kind(), error.status(), error.error_type()),
        (ErrorKind::BadRequest, Some(400), Some("api_usage_error")),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "calls the real TypeSafe API and costs money: see the module documentation"]
async fn the_server_still_silently_accepts_an_unknown_field_inside_a_question() {
    let Some(key) = key() else { return };
    let transport = transport(&key);
    // A misspelt `criteria`: the server answers as if it were not there.
    let question = json!({
        "type": "noul",
        "instructions": "Did the parcel arrive intact?",
        "critera": { "true": "It arrived undamaged", "false": "It was damaged" }
    });
    let answer = answer(&transport, &request(&question), "unknown question field").await;
    assert_noul(&answer);
}
