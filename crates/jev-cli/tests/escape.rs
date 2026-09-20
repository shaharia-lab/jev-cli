//! Terminal escape sequences in text `jev` did not write itself never reach a person's terminal.
//!
//! A server's answer, a model card, a question id echoed from a request file and a batch row are
//! all untrusted (CWE-150). Text output neutralises their control characters; JSON keeps them, so
//! a program reading stdout gets the value it was sent.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use assert_cmd::Command;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A key that must never appear in any output. It is not a real credential.
const SENTINEL_KEY: &str = "sentinel-key-do-not-leak-3d90";

/// What an attacker hides in a value: text made invisible, a window title, and a carriage return
/// that rewrites the line already printed to its left.
const SENTINEL: &str = "\u{1b}[8mhidden\u{1b}]0;title\u{7}\rrewrite";

/// The same text once `jev` has written its control characters out as escapes.
const ESCAPED: &str = "\\u{1b}[8mhidden\\u{1b}]0;title\\u{7}\\u{d}rewrite";

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
    // An address nothing listens on, so a test that forgets its mock cannot reach the real API.
    command.env(
        "TYPESAFE_BASE_URL",
        server.map_or_else(|| "http://127.0.0.1:9".to_owned(), MockServer::uri),
    );
    command
}

/// `jev`, pointed at `server`, with `arguments`.
fn command(server: Option<&MockServer>, arguments: &[&str]) -> Command {
    let mut command = jev(server);
    command.args(arguments);
    command
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run_blocking(mut command: Command) -> Run {
    let output = command.timeout(Duration::from_secs(60)).output().unwrap();
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

/// Runs `jev` off the async runtime's threads, since the mock server lives on them.
async fn run(command: Command) -> Run {
    tokio::task::spawn_blocking(move || run_blocking(command))
        .await
        .unwrap()
}

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("not JSON ({error}): {text:?}"))
}

/// Writes `content` where a test can point `jev` at it.
fn written(name: &str, content: &Value) -> String {
    let path: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("escape")
        .join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, serde_json::to_vec(content).unwrap()).unwrap();
    path.to_str().unwrap().to_owned()
}

/// The first character that could drive a terminal: C0 bar tab and newline, DEL, or C1.
fn drives_a_terminal(text: &str) -> Option<char> {
    text.chars()
        .find(|&c| c.is_control() && c != '\n' && c != '\t')
}

/// Asserts that nothing `jev` printed for a person can drive the terminal, and that stdout still
/// shows the sentinel — escaped, not dropped, so a person sees that something was there.
fn assert_neutralised(run: &Run) {
    for (stream, text) in [("stdout", &run.stdout), ("stderr", &run.stderr)] {
        assert!(
            drives_a_terminal(text).is_none(),
            "{stream} holds {:?} in {text:?}",
            drives_a_terminal(text)
        );
    }
    assert!(
        run.stdout.contains(ESCAPED),
        "the sentinel was dropped rather than escaped: {:?}",
        run.stdout
    );
}

#[tokio::test]
async fn a_model_card_from_the_server_is_escaped_in_the_table_and_kept_in_json() {
    let server = MockServer::start().await;
    let body = json!({ "models": [
        {
            "name": format!("jev-{SENTINEL}"),
            "description": format!("a description {SENTINEL}"),
            "release_date": "2026-09-10T00:00:00Z",
        },
        { "name": "jev-plain", "description": "plain", "release_date": "2026-09-11T00:00:00Z" },
    ]});
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let table = run(command(Some(&server), &["models", "list", "-o", "table"])).await;

    assert_eq!(table.code, 0, "{:?}", table.stderr);
    assert_neutralised(&table);
    // The escaped text is wider than the raw text, and the columns are laid out around it: the
    // date still starts at the same place on every row.
    let released: Vec<usize> = table
        .stdout
        .lines()
        .map(|line| {
            line.find("2026-09-1")
                .unwrap_or_else(|| line.find("RELEASED").unwrap())
        })
        .collect();
    assert!(
        released.windows(2).all(|pair| pair[0] == pair[1]),
        "the columns do not line up: {released:?} in {:?}",
        table.stdout
    );

    let machine = run(command(Some(&server), &["models", "list", "-o", "json"])).await;

    assert_eq!(machine.code, 0, "{:?}", machine.stderr);
    assert_eq!(
        json_of(&machine.stdout)["models"][0]["description"],
        body["models"][0]["description"],
        "JSON must hand the value over unchanged"
    );
    assert!(
        !machine.stdout.contains('\u{1b}'),
        "JSON escapes control characters itself: {:?}",
        machine.stdout
    );
}

#[tokio::test]
async fn an_answer_from_the_server_is_escaped_in_the_text_form_and_kept_in_json() {
    let server = MockServer::start().await;
    // Every string in an answer comes back from the server: the question id, the chosen option,
    // an option in the distribution, a score legend and the model that answered.
    let answers = json!({
        format!("is_urgent {SENTINEL}"): { "type": "noul", "noul": 0.92 },
        "department": {
            "type": "choice",
            "choice": format!("technical {SENTINEL}"),
            "confidence": 0.8,
            "probabilities": { format!("technical {SENTINEL}"): 0.9, "billing": 0.1 },
        },
        "frustration": {
            "type": "score",
            "score": 1.0,
            "confidence": 0.7,
            "legend": { "0": "calm", "1": format!("angry {SENTINEL}") },
            "probabilities": { "0": 0.3, "1": 0.7 },
        },
    });
    let body = json!({
        "model": format!("jev-1.13.0 {SENTINEL}"),
        "answers": answers,
        "usage": { "input_tokens": 12, "output_tokens": 0 },
    });
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;
    let request = written(
        "request.json",
        &json!({
            "model": "jev-latest",
            "state": "a ticket",
            "questions": { "is_urgent": { "type": "noul", "instructions": "Is it urgent?" } },
        }),
    );

    let table = run(command(
        Some(&server),
        &["eval", "-f", &request, "-o", "table"],
    ))
    .await;

    assert_eq!(table.code, 0, "{:?}", table.stderr);
    assert_neutralised(&table);

    let machine = run(command(
        Some(&server),
        &["eval", "-f", &request, "-o", "json"],
    ))
    .await;

    assert_eq!(machine.code, 0, "{:?}", machine.stderr);
    assert_eq!(
        json_of(&machine.stdout)["answers"],
        answers,
        "answers are passed through byte for byte"
    );
}

#[tokio::test]
async fn the_diagnostic_stream_escapes_what_the_server_sent_under_double_verbose() {
    let server = MockServer::start().await;
    // A gateway between `jev` and the API answers with its own error page, so the bytes are not
    // JSON and nothing has escaped the control characters in them on the way.
    let page = format!("<html><body>502 Bad Gateway {SENTINEL}</body></html>");
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(502)
                .insert_header("content-type", "text/html")
                .set_body_string(&page),
        )
        .mount(&server)
        .await;
    let request = written(
        "verbose.json",
        &json!({
            "model": "jev-latest",
            "state": "a ticket",
            "questions": { "is_urgent": { "type": "noul", "instructions": "Is it urgent?" } },
        }),
    );

    // `-vv` logs every crate at debug; `--debug-bodies` is what puts the body on stderr at all.
    let run = run(command(
        Some(&server),
        &[
            "-vv",
            "--debug-bodies",
            "--max-retries",
            "0",
            "eval",
            "-f",
            &request,
            "-o",
            "table",
        ],
    ))
    .await;

    assert_eq!(run.code, 6, "{:?}", run.stderr);
    assert!(
        drives_a_terminal(&run.stderr).is_none(),
        "the diagnostic stream holds {:?} in {:?}",
        drives_a_terminal(&run.stderr),
        run.stderr
    );
    assert!(
        run.stderr.contains(ESCAPED),
        "the body was dropped rather than escaped: {:?}",
        run.stderr
    );
    // The level, the target and the message still read as they did.
    assert!(
        run.stderr.contains("jev_client::http::body")
            && run.stderr.contains("response body")
            && run.stderr.contains("502 Bad Gateway"),
        "the diagnostic line is no longer readable: {:?}",
        run.stderr
    );
}

#[test]
fn a_validation_finding_escapes_the_document_it_quotes() {
    // An unknown top-level field is reported by name, and the name is whatever the file holds.
    let request = written(
        "invalid.json",
        &json!({
            "model": "jev-latest",
            "state": "a ticket",
            format!("stowaway {SENTINEL}"): 1,
            "questions": { "is_urgent": { "type": "noul", "instructions": "Is it urgent?" } },
        }),
    );

    let run = run_blocking(command(None, &["validate", "-f", &request, "-o", "table"]));

    assert_eq!(run.code, 2, "{:?}", run.stderr);
    assert_neutralised(&run);
}

#[test]
fn a_dry_run_escapes_the_model_name_the_request_file_asks_for() {
    let request = written(
        "dry-run.json",
        &json!({
            "model": format!("jev-latest {SENTINEL}"),
            "state": "a ticket",
            "questions": { "is_urgent": { "type": "noul", "instructions": "Is it urgent?" } },
        }),
    );

    let run = run_blocking(command(
        None,
        &["eval", "-f", &request, "--dry-run", "-o", "table"],
    ));

    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_neutralised(&run);
}

#[test]
fn a_profile_name_from_the_config_file_is_escaped_where_it_is_confirmed() {
    // `jev profile create` validates a name, but a hand-written config file can hold anything.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("escape-config");
    fs::create_dir_all(&dir).unwrap();
    let name = format!("work {SENTINEL}");
    fs::write(
        dir.join("config.toml"),
        format!(
            "active_profile = {}\n\n[profiles.{}]\n",
            serde_json::to_string(&name).unwrap(),
            serde_json::to_string(&name).unwrap()
        ),
    )
    .unwrap();

    let mut login = command(
        None,
        &[
            "auth",
            "login",
            "--with-token",
            "--skip-verify",
            "-o",
            "table",
        ],
    );
    login.env("JEV_CONFIG_DIR", &dir);
    login.env_remove("TYPESAFE_API_KEY");
    login.write_stdin("a-key-that-is-not-real-0000");
    let run = run_blocking(login);

    assert_eq!(run.code, 0, "{:?}", run.stderr);
    assert_neutralised(&run);
}

#[test]
fn a_batch_plan_escapes_the_row_id_it_quotes() {
    let questions = written(
        "batch-questions.json",
        &json!({
            "questions": { "is_urgent": { "type": "noul", "instructions": "Is it urgent?" } },
        }),
    );
    let row = json!({ "id": format!("row {SENTINEL}"), "text": "a ticket" });
    let rows: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("escape")
        .join("rows.jsonl");
    // The same id twice, so the plan reports it as a duplicate and quotes it.
    fs::write(&rows, format!("{row}\n{row}\n")).unwrap();

    let run = run_blocking(command(
        None,
        &[
            "batch",
            "run",
            "-f",
            &questions,
            "--input",
            rows.to_str().unwrap(),
            "--id-field",
            "id",
            "--dry-run",
            "-o",
            "table",
        ],
    ));

    // The plan is printed first, listing the duplicate; the run then exits 2 without sending.
    assert_eq!(run.code, 2, "{:?}", run.stderr);
    assert_neutralised(&run);
}
