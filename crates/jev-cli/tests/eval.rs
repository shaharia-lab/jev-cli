//! `jev eval` and `jev models list` against a local mock of the API.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::Path;
use std::time::Duration;

use assert_cmd::Command;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A key that must never appear in any output. It is not a real credential.
const SENTINEL_KEY: &str = "sentinel-key-do-not-leak-41c7";

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/eval")
        .join(name)
        .to_str()
        .unwrap()
        .to_owned()
}

fn fixture_json(name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(fixture(name)).unwrap()).unwrap()
}

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
    // Never the real keychain of whoever runs the tests.
    command.env("JEV_NO_KEYCHAIN", "1");
    command.env("TYPESAFE_API_KEY", SENTINEL_KEY);
    // An address nothing listens on, so a test that forgets its mock cannot reach the real API.
    command.env(
        "TYPESAFE_BASE_URL",
        server.map_or_else(|| "http://127.0.0.1:9".to_owned(), MockServer::uri),
    );
    command
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Runs `jev` off the async runtime's threads, since the mock server lives on them.
async fn run(mut command: Command) -> Run {
    tokio::task::spawn_blocking(move || {
        let output = command.timeout(Duration::from_secs(60)).output().unwrap();
        Run {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    })
    .await
    .unwrap()
}

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("not JSON ({error}): {text:?}"))
}

fn success() -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("x-typesafe-request-id", "req_ok")
        .set_body_json(fixture_json("response.json"))
}

fn api_error(status: u16, error_type: &str, message: &str) -> ResponseTemplate {
    ResponseTemplate::new(status)
        .insert_header("x-typesafe-request-id", format!("req_{status}"))
        .set_body_json(json!({ "detail": { "error_type": error_type, "message": message } }))
}

async fn mount(server: &MockServer, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(response)
        .mount(server)
        .await;
}

async fn received(server: &MockServer) -> Vec<wiremock::Request> {
    server.received_requests().await.unwrap()
}

fn assert_no_key(run: &Run) {
    assert!(
        !run.stdout.contains(SENTINEL_KEY),
        "the key leaked to stdout: {}",
        run.stdout
    );
    assert!(
        !run.stderr.contains(SENTINEL_KEY),
        "the key leaked to stderr: {}",
        run.stderr
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn all_three_question_types_are_answered_in_one_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(header(
            "authorization",
            format!("Bearer {SENTINEL_KEY}").as_str(),
        ))
        .respond_with(success())
        .expect(1)
        .mount(&server)
        .await;
    let mut command = jev(Some(&server));
    command.args(["eval", "-f", &fixture("triage.yaml")]);

    let run = run(command).await;

    assert_eq!((run.code, run.stderr.as_str()), (0, ""), "{}", run.stdout);
    let result = json_of(&run.stdout);
    let keys: Vec<&str> = result
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "model",
            "requested_model",
            "answers",
            "usage",
            "cost_usd",
            "request_id",
            "latency_ms"
        ]
    );
    assert_eq!(result["model"], "jev-1.13.0");
    assert_eq!(result["requested_model"], "jev-latest");
    assert_eq!(
        result["answers"],
        fixture_json("response.json")["answers"],
        "answers pass through unmodified"
    );
    assert_eq!(
        result["usage"],
        json!({ "input_tokens": 459, "output_tokens": 73 })
    );
    assert!((result["cost_usd"].as_f64().unwrap() - 0.000_019_278).abs() < 1e-12);
    assert_eq!(result["request_id"], "req_ok");
    assert!(result["latency_ms"].is_u64());
    assert_no_key(&run);

    let agent = received(&server).await[0]
        .headers
        .get("user-agent")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        agent.starts_with(concat!("jev/", env!("CARGO_PKG_VERSION"), " jev-client/")),
        "{agent}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_yaml_request_and_its_json_equivalent_send_byte_identical_bodies() {
    let server = MockServer::start().await;
    mount(&server, success()).await;

    for file in ["triage.yaml", "triage.json"] {
        let mut command = jev(Some(&server));
        command.args(["eval", "-f", &fixture(file)]);
        assert_eq!(run(command).await.code, 0, "{file}");
    }

    let requests = received(&server).await;
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].body, requests[1].body,
        "YAML and JSON must produce the same bytes"
    );
    let sent: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        sent,
        fixture_json("triage.json"),
        "and the JSON form is the API body itself"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_human_result_shows_the_alias_the_resolved_model_and_the_cost() {
    let server = MockServer::start().await;
    mount(&server, success()).await;
    let mut command = jev(Some(&server));
    command.args([
        "eval",
        "-f",
        &fixture("triage.yaml"),
        "-o",
        "table",
        "--ascii",
    ]);

    let run = run(command).await;

    assert_eq!(run.code, 0, "{}", run.stderr);
    for expected in [
        "is_urgent  noul  yes  0.94",
        "department  choice  billing  (confidence 0.89)",
        "    billing    0.93  ###################.",
        "frustration  score  1.20  (levels 0 to 2, confidence 0.68)",
        "model jev-1.13.0 (requested jev-latest) | 459 input tokens | est. cost $0.000019 |",
        "request req_ok",
    ] {
        assert!(
            run.stdout.contains(expected),
            "missing {expected:?} in:\n{}",
            run.stdout
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn api_failures_exit_with_their_documented_codes() {
    let cases = [
        (
            api_error(401, "authentication_error", "Invalid API key."),
            3,
            "authentication",
            false,
        ),
        (
            api_error(422, "invalid_request_error", "Bad question."),
            4,
            "api_rejected",
            false,
        ),
        (
            api_error(429, "rate_limit_error", "Slow down.").insert_header("retry-after-ms", "1"),
            5,
            "rate_limited",
            true,
        ),
    ];
    for (response, exit, code, retried) in cases {
        let server = MockServer::start().await;
        mount(&server, response).await;
        let mut command = jev(Some(&server));
        command.args(["eval", "-f", &fixture("triage.json"), "--max-retries", "2"]);

        let run = run(command).await;

        assert_eq!(
            (run.code, run.stdout.as_str()),
            (exit, ""),
            "{code}: {}",
            run.stderr
        );
        let error = &json_of(&run.stderr)["error"];
        assert_eq!(error["code"], code);
        assert_eq!(error["exit_code"], exit);
        assert_eq!(error["request_id"], format!("req_{}", error["http_status"]));
        assert!(error["hint"].as_str().is_some_and(|hint| !hint.is_empty()));
        assert_eq!(
            received(&server).await.len(),
            if retried { 3 } else { 1 },
            "{code}"
        );
        assert_eq!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("after 3 attempts"),
            retried,
            "{error}"
        );
        assert_no_key(&run);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_timeout_exits_6() {
    let server = MockServer::start().await;
    mount(&server, success().set_delay(Duration::from_secs(20))).await;
    let mut command = jev(Some(&server));
    command.args([
        "eval",
        "-f",
        &fixture("triage.json"),
        "--timeout",
        "200ms",
        "--max-retries",
        "0",
    ]);

    let run = run(command).await;

    assert_eq!((run.code, run.stdout.as_str()), (6, ""), "{}", run.stderr);
    let error = &json_of(&run.stderr)["error"];
    assert_eq!(error["code"], "timeout");
    assert_eq!(error["retryable"], true);
    assert!(
        error["hint"].as_str().unwrap().contains("--timeout"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_key_exits_3_naming_both_remedies_and_sends_nothing() {
    let server = MockServer::start().await;
    mount(&server, success()).await;
    let mut command = jev(Some(&server));
    command
        .env_remove("TYPESAFE_API_KEY")
        .args(["eval", "-f", &fixture("triage.json")]);

    let run = run(command).await;

    assert_eq!((run.code, run.stdout.as_str()), (3, ""));
    let error = &json_of(&run.stderr)["error"];
    assert_eq!(error["code"], "no_api_key");
    let hint = error["hint"].as_str().unwrap();
    assert!(
        hint.contains("TYPESAFE_API_KEY") && hint.contains("jev auth login"),
        "{hint}"
    );
    assert!(received(&server).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn piped_stdin_is_the_state_when_the_file_has_none() {
    let server = MockServer::start().await;
    mount(&server, success()).await;
    let mut command = jev(Some(&server));
    command
        .args(["eval", "-f", &fixture("questions.yaml")])
        .write_stdin("Help! My payouts have been failing.\n");

    let run = run(command).await;

    assert_eq!(run.code, 0, "{}", run.stderr);
    let sent: Value = serde_json::from_slice(&received(&server).await[0].body).unwrap();
    assert_eq!(sent["state"], "Help! My payouts have been failing.\n");
    assert_eq!(
        sent["model"], "jev-latest",
        "the default model is filled in"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_state_can_come_from_a_flag_a_file_or_an_explicit_pipe_and_json_stays_structured() {
    let server = MockServer::start().await;
    mount(&server, success()).await;
    let object = r#"{"ticket": {"plan": "pro"}}"#;
    let cases: [(Vec<&str>, Option<&str>, Value); 4] = [
        (vec!["--state", "from a flag"], None, json!("from a flag")),
        (
            vec!["--state", object, "--state-format", "json"],
            None,
            json!({ "ticket": { "plan": "pro" } }),
        ),
        (
            vec!["--state-file", "-", "--state-format", "json"],
            Some(object),
            json!({ "ticket": { "plan": "pro" } }),
        ),
        // The file has a state of its own; a flag replaces it.
        (vec!["--state", "override"], None, json!("override")),
    ];

    for (index, (arguments, stdin, expected)) in cases.into_iter().enumerate() {
        let file = if index == 3 {
            fixture("triage.yaml")
        } else {
            fixture("questions.yaml")
        };
        let mut command = jev(Some(&server));
        command.args(["eval", "-f", &file]).args(&arguments);
        if let Some(stdin) = stdin {
            command.write_stdin(stdin);
        }
        let run = run(command).await;

        assert_eq!(run.code, 0, "{arguments:?}: {}", run.stderr);
        let sent: Value = serde_json::from_slice(&received(&server).await[index].body).unwrap();
        assert_eq!(sent["state"], expected, "{arguments:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_with_its_own_state_does_not_wait_on_an_open_stdin() {
    let server = MockServer::start().await;
    mount(&server, success()).await;
    // An agent often leaves stdin open and never writes to it. `jev` must not block on it.
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("jev"))
        .args(["eval", "-f", &fixture("triage.json")])
        .env(
            "JEV_CONFIG_DIR",
            Path::new(env!("CARGO_TARGET_TMPDIR")).join("no-config"),
        )
        .env("TYPESAFE_API_KEY", SENTINEL_KEY)
        .env("TYPESAFE_BASE_URL", server.uri())
        .env_remove("JEV_PROFILE")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let held_open = child.stdin.take().unwrap();

    let output = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::task::spawn_blocking(move || child.wait_with_output().unwrap()),
    )
    .await
    .expect("jev blocked on a stdin nobody writes to")
    .unwrap();

    drop(held_open);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ambiguous_or_missing_state_is_a_usage_error_before_anything_is_sent() {
    let server = MockServer::start().await;
    mount(&server, success()).await;
    let questions = std::fs::read_to_string(fixture("questions.yaml")).unwrap();
    let cases: [(Vec<String>, Option<&str>, &str); 3] = [
        (
            vec!["-f".into(), "-".into()],
            Some(&questions),
            "stdin is already being read as the request file",
        ),
        (
            vec!["-f".into(), "-".into(), "--state-file".into(), "-".into()],
            Some(&questions),
            "stdin is already being read as the request file",
        ),
        (
            vec![
                "-f".into(),
                fixture("questions.yaml"),
                "--state".into(),
                "a".into(),
                "--state-file".into(),
                "b".into(),
            ],
            None,
            "cannot be used with",
        ),
    ];

    for (arguments, stdin, message) in cases {
        let mut command = jev(Some(&server));
        command.arg("eval").args(&arguments);
        if let Some(stdin) = stdin {
            command.write_stdin(stdin);
        }
        let run = run(command).await;

        assert_eq!((run.code, run.stdout.as_str()), (2, ""), "{arguments:?}");
        assert!(
            json_of(&run.stderr)["error"]["message"]
                .as_str()
                .unwrap()
                .contains(message),
            "{}",
            run.stderr
        );
    }
    assert!(received(&server).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_file_on_stdin_works_when_it_carries_its_own_state() {
    let server = MockServer::start().await;
    mount(&server, success()).await;
    let mut command = jev(Some(&server));
    command
        .args(["eval", "-f", "-"])
        .write_stdin(std::fs::read_to_string(fixture("triage.yaml")).unwrap());

    let run = run(command).await;

    assert_eq!(run.code, 0, "{}", run.stderr);
    let sent: Value = serde_json::from_slice(&received(&server).await[0].body).unwrap();
    assert_eq!(sent, fixture_json("triage.json"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invalid_request_exits_2_before_any_http_request() {
    let server = MockServer::start().await;
    mount(&server, success()).await;
    let mut machine = jev(Some(&server));
    machine.args(["eval", "-f", &fixture("eleven-levels.json")]);
    let mut person = jev(Some(&server));
    person.args(["eval", "-f", &fixture("eleven-levels.json"), "-o", "table"]);

    let (machine, person) = (run(machine).await, run(person).await);

    assert_eq!((machine.code, machine.stdout.as_str()), (2, ""));
    let error = &json_of(&machine.stderr)["error"];
    assert_eq!(
        error["message"],
        "the request is not valid: 1 problem found, nothing was sent"
    );
    assert_eq!(
        error["details"]["findings"][0]["rule"],
        "score-too-many-levels"
    );
    assert_eq!(error["details"]["findings"][0]["question"], "rating");
    assert_eq!(
        error["details"]["findings"][0]["path"],
        "/questions/rating/criteria"
    );

    assert_eq!(person.code, 2);
    assert!(
        person
            .stderr
            .contains("- /questions/rating/criteria: score `rating` has 11 levels"),
        "{}",
        person.stderr
    );
    assert!(
        person.stderr.contains("  fix: merge neighbouring levels"),
        "{}",
        person.stderr
    );
    assert!(
        received(&server).await.is_empty(),
        "nothing may be sent for an invalid request"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dry_run_prints_the_exact_body_sends_nothing_and_needs_no_key() {
    let server = MockServer::start().await;
    mount(&server, success()).await;
    let mut with_key = jev(Some(&server));
    with_key.args([
        "eval",
        "-f",
        &fixture("triage.yaml"),
        "--dry-run",
        "--model",
        "jev-1.13.0",
    ]);
    let mut without_key = jev(Some(&server));
    without_key.env_remove("TYPESAFE_API_KEY").args([
        "eval",
        "-f",
        &fixture("triage.yaml"),
        "--dry-run",
        "-o",
        "table",
    ]);

    let (with_key, without_key) = (run(with_key).await, run(without_key).await);

    assert_eq!(
        (with_key.code, with_key.stderr.as_str()),
        (0, ""),
        "{}",
        with_key.stdout
    );
    let dry_run = json_of(&with_key.stdout);
    let mut expected_body = fixture_json("triage.json");
    expected_body["model"] = json!("jev-1.13.0");
    assert_eq!(dry_run["dry_run"], true);
    assert_eq!(dry_run["body"], expected_body);
    assert_eq!(
        dry_run["endpoint"],
        format!("{}/v1/systemone", server.uri())
    );
    assert_eq!(
        (
            dry_run["requested_model"].as_str(),
            dry_run["model_origin"].as_str()
        ),
        (Some("jev-1.13.0"), Some("--model"))
    );
    assert!(dry_run["size_estimate"]["total_tokens"].as_u64().unwrap() > 270);
    assert_no_key(&with_key);

    assert_eq!(
        without_key.code, 0,
        "a dry run needs no key: {}",
        without_key.stderr
    );
    assert!(
        without_key
            .stdout
            .starts_with("dry run: nothing was sent\nPOST "),
        "{}",
        without_key.stdout
    );
    assert!(
        without_key
            .stdout
            .contains("model jev-latest (from the request file)"),
        "{}",
        without_key.stdout
    );
    assert!(
        without_key.stdout.contains("estimated input tokens: "),
        "{}",
        without_key.stdout
    );

    assert!(
        received(&server).await.is_empty(),
        "a dry run makes no HTTP request"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_prints_the_response_body_exactly_as_received() {
    let server = MockServer::start().await;
    let body = r#"{"model":"jev-1.13.0",  "answers":{"q":{"type":"noul","noul":0.5,"future":true}},"usage":{"input_tokens":1,"output_tokens":1}}"#;
    mount(
        &server,
        ResponseTemplate::new(200).set_body_raw(body, "application/json"),
    )
    .await;
    let mut command = jev(Some(&server));
    command.args(["eval", "-f", &fixture("triage.json"), "--raw"]);

    let run = run(command).await;

    assert_eq!(
        (run.code, run.stdout.as_str()),
        (0, format!("{body}\n").as_str())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn warnings_go_to_stderr_and_the_result_still_arrives() {
    let server = MockServer::start().await;
    mount(&server, success()).await;
    let request = json!({ "state": "s", "questions": { "team": {
        "type": "choice", "instructions": "Which team?", "criteria": { "billing": "a", "technical": "b", "sales": "c" }
    }}});
    let mut relaxed = jev(Some(&server));
    relaxed
        .args(["eval", "-f", "-", "--warn-unpinned"])
        .write_stdin(request.to_string());
    let mut strict = jev(Some(&server));
    strict
        .args(["eval", "-f", "-", "--strict"])
        .write_stdin(request.to_string());

    let (relaxed, strict) = (run(relaxed).await, run(strict).await);

    assert_eq!(relaxed.code, 0);
    assert_eq!(json_of(&relaxed.stdout)["model"], "jev-1.13.0");
    let warnings: Vec<Value> = relaxed.stderr.lines().map(json_of).collect();
    let codes: Vec<&str> = warnings
        .iter()
        .map(|warning| warning["warning"]["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["request_lint", "unpinned_model"]);
    assert!(
        warnings[0]["warning"]["message"]
            .as_str()
            .unwrap()
            .contains("[choice-no-escape-option]")
    );
    assert!(
        warnings[1]["warning"]["hint"]
            .as_str()
            .unwrap()
            .contains("--model jev-1.13.0")
    );

    assert_eq!(
        (strict.code, strict.stdout.as_str()),
        (2, ""),
        "--strict refuses a request with warnings"
    );
    assert_eq!(
        received(&server).await.len(),
        1,
        "only the relaxed run reached the API"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_http_to_another_host_is_refused_unless_explicitly_allowed() {
    let mut refused = jev(None);
    refused
        .env("TYPESAFE_BASE_URL", "http://example.invalid")
        .args(["eval", "-f", &fixture("triage.json")]);
    let mut allowed = jev(None);
    allowed
        .env("TYPESAFE_BASE_URL", "http://example.invalid")
        .args([
            "eval",
            "-f",
            &fixture("triage.json"),
            "--insecure-allow-http",
            "--max-retries",
            "0",
            "--timeout",
            "2",
        ]);

    let (refused, allowed) = (run(refused).await, run(allowed).await);

    assert_eq!(refused.code, 2);
    let error = &json_of(&refused.stderr)["error"];
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .starts_with("the base URL from TYPESAFE_BASE_URL is not acceptable"),
        "{error}"
    );
    assert!(
        error["hint"]
            .as_str()
            .unwrap()
            .contains("--insecure-allow-http"),
        "{error}"
    );

    assert_eq!(
        allowed.code, 6,
        "allowed, and then the host does not exist: {}",
        allowed.stderr
    );
    let first: Value = json_of(allowed.stderr.lines().next().unwrap());
    assert_eq!(first["warning"]["code"], "insecure_http");
    assert_no_key(&allowed);
}

#[tokio::test(flavor = "multi_thread")]
async fn models_are_listed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "models": [
            { "name": "jev-latest", "description": "The latest Jev", "release_date": "2026-09-10T18:38:01.391457+00:00" },
            { "name": "jev-preview", "description": "A preview", "release_date": "2026-09-10T18:39:06.057655+00:00" }
        ]})))
        .mount(&server)
        .await;
    let mut machine = jev(Some(&server));
    machine.args(["models", "list"]);
    let mut person = jev(Some(&server));
    person.args(["models", "list", "-o", "table"]);
    let mut lines = jev(Some(&server));
    lines.args(["models", "list", "-o", "jsonl"]);

    let (machine, person, lines) = (run(machine).await, run(person).await, run(lines).await);

    assert_eq!(machine.code, 0, "{}", machine.stderr);
    assert_eq!(json_of(&machine.stdout)["models"][1]["name"], "jev-preview");
    assert!(
        person
            .stdout
            .contains("NAME         RELEASED    DESCRIPTION"),
        "{}",
        person.stdout
    );
    assert!(
        person
            .stdout
            .contains("jev-latest   2026-09-10  The latest Jev"),
        "{}",
        person.stdout
    );
    assert_eq!(lines.stdout.lines().count(), 2, "one record per model");
    assert_no_key(&machine);
}

#[tokio::test(flavor = "multi_thread")]
async fn help_explains_model_resolution_and_that_aliases_move() {
    let mut eval = jev(None);
    eval.args(["eval", "--help"]);
    let mut models = jev(None);
    models.args(["models", "list", "--help"]);

    let (eval, models) = (run(eval).await, run(models).await);

    assert!(
        eval.stdout.contains("--dry-run") && eval.stdout.contains("--state-file"),
        "{}",
        eval.stdout
    );
    assert!(
        models
            .stdout
            .contains("accepted by --model even when it is not listed"),
        "{}",
        models.stdout
    );
    assert!(
        models
            .stdout
            .contains("move to newer versions without notice"),
        "{}",
        models.stdout
    );
}
