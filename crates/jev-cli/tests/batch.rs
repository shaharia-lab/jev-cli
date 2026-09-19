//! `jev batch run`, against a local mock of the API.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use assert_cmd::Command;
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SENTINEL_KEY: &str = "sentinel-key-do-not-leak-7a1e";

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
    command
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

async fn run(mut command: Command) -> Run {
    tokio::task::spawn_blocking(move || {
        let output = command.timeout(Duration::from_secs(120)).output().unwrap();
        let run = Run {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        };
        assert!(
            !run.stdout.contains(SENTINEL_KEY) && !run.stderr.contains(SENTINEL_KEY),
            "the API key leaked"
        );
        run
    })
    .await
    .unwrap()
}

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("not JSON ({error}): {text:?}"))
}

fn lines_of(text: &str) -> Vec<Value> {
    text.lines().map(json_of).collect()
}

/// A directory of its own for one test's files.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("batch-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, content: &str) -> String {
    let file = dir.join(name);
    fs::write(&file, content).unwrap();
    file.to_str().unwrap().to_owned()
}

const QUESTIONS: &str = "questions:\n  urgent:\n    type: noul\n    instructions: Is it urgent?\n";

fn answer() -> Value {
    json!({ "model": "jev-1.13.0", "answers": { "urgent": { "type": "noul", "noul": 0.9 } }, "usage": { "input_tokens": 300, "output_tokens": 20 } })
}

/// A mock that answers every row, and fails any row whose state contains `FAIL` with a 400.
async fn mock(recording: bool) -> MockServer {
    let server = if recording {
        MockServer::start().await
    } else {
        MockServer::builder()
            .disable_request_recording()
            .start()
            .await
    };
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(body_string_contains("FAIL"))
        .respond_with(
            ResponseTemplate::new(400)
                .insert_header("x-typesafe-request-id", "req_bad")
                .set_body_json(json!({ "detail": { "error_type": "invalid_request_error", "message": "The state is not acceptable." } })),
        )
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-typesafe-request-id", "req_ok")
                .set_body_json(answer()),
        )
        .mount(&server)
        .await;
    server
}

async fn sent(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| serde_json::from_slice(&request.body).unwrap())
        .collect()
}

/// JSONL rows `{"n": 1, "text": ...}`, where the rows in `failing` carry `FAIL`.
fn rows(count: u64, failing: &[u64]) -> String {
    (1..=count).fold(String::new(), |mut text, n| {
        let body = if failing.contains(&n) { "FAIL" } else { "fine" };
        let _ = writeln!(text, "{{\"n\": {n}, \"text\": \"{body} {n}\"}}");
        text
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn ten_thousand_rows_give_ten_thousand_records_and_an_accurate_summary() {
    let server = mock(false).await;
    let dir = scratch("ten-thousand");
    let questions = write(&dir, "questions.yaml", QUESTIONS);
    let input = write(&dir, "rows.jsonl", &rows(10_000, &[]));
    let out = dir.join("results.jsonl");
    let summary_path = dir.join("summary.json");
    let mut command = jev(Some(&server));
    command.args([
        "batch",
        "run",
        "-f",
        &questions,
        "--input",
        &input,
        "--state-field",
        "text",
    ]);
    command.args(["--concurrency", "8", "--out"]).arg(&out);
    command.arg("--summary-json").arg(&summary_path);

    let run = run(command).await;

    assert_eq!((run.code, run.stdout.as_str()), (0, ""), "{}", run.stderr);
    let records = lines_of(&fs::read_to_string(&out).unwrap());
    assert_eq!(records.len(), 10_000);
    let ids: BTreeSet<u64> = records
        .iter()
        .map(|record| record["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, (1..=10_000).collect());
    let record = &records[0];
    let keys: Vec<&str> = record
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "id",
            "status",
            "model",
            "answers",
            "usage",
            "cost_usd",
            "request_id",
            "latency_ms"
        ]
    );
    assert_eq!(record["status"], "ok");
    assert_eq!(record["model"], "jev-1.13.0");
    assert_eq!(record["answers"]["urgent"]["noul"], 0.9);
    assert_eq!(record["request_id"], "req_ok");

    let summary = json_of(&fs::read_to_string(&summary_path).unwrap());
    assert_eq!(
        (
            &summary["rows_total"],
            &summary["ok"],
            &summary["failed"],
            &summary["skipped"]
        ),
        (&json!(10_000), &json!(10_000), &json!(0), &json!(0))
    );
    assert_eq!(summary["input_tokens"], 3_000_000);
    assert!(
        (summary["cost_usd"].as_f64().unwrap() - 0.126).abs() < 1e-9,
        "{summary}"
    );
    assert_eq!(summary["retries"], 0);
    assert_eq!(summary["models"], json!(["jev-1.13.0"]));
    assert_eq!(summary["stopped_by"], Value::Null);
    assert!(summary["rows_per_second"].as_f64().unwrap() > 0.0);
    // Piped, so the summary on stderr is one JSON line, the same as the file.
    assert_eq!(run.stderr.lines().count(), 1, "{}", run.stderr);
    assert_eq!(json_of(&run.stderr)["summary"]["ok"], 10_000);
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_rows_are_recorded_with_the_api_error_and_the_run_exits_7() {
    let server = mock(true).await;
    let dir = scratch("failures");
    let questions = write(&dir, "questions.yaml", QUESTIONS);
    let input = write(&dir, "rows.jsonl", &rows(10, &[3, 7]));
    let mut command = jev(Some(&server));
    command.args([
        "batch",
        "run",
        "-f",
        &questions,
        "--input",
        &input,
        "--state-field",
        "text",
    ]);

    let run = run(command).await;

    assert_eq!(run.code, 7, "{}", run.stderr);
    let records = lines_of(&run.stdout);
    assert_eq!(records.len(), 10);
    let failed: Vec<&Value> = records
        .iter()
        .filter(|record| record["status"] == "error")
        .collect();
    assert_eq!(failed.len(), 2);
    for record in failed {
        assert!([json!(3), json!(7)].contains(&record["id"]), "{record}");
        let keys: Vec<&str> = record
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["id", "status", "error"]);
        let error = &record["error"];
        assert_eq!(error["code"], "api_rejected");
        assert_eq!(error["exit_code"], 4);
        assert_eq!(error["error_type"], "invalid_request_error");
        assert_eq!(error["request_id"], "req_bad");
        assert_eq!(error["http_status"], 400);
    }
    assert_eq!(sent(&server).await.len(), 10, "every row was tried");
    let summary = &json_of(&run.stderr)["summary"];
    assert_eq!((&summary["ok"], &summary["failed"]), (&json!(8), &json!(2)));
}

#[tokio::test(flavor = "multi_thread")]
async fn fail_fast_and_max_errors_stop_sending() {
    let dir = scratch("stopping");
    let questions = write(&dir, "questions.yaml", QUESTIONS);
    let input = write(&dir, "rows.jsonl", &rows(10, &[2, 4, 6]));

    for (flags, sent_rows, stopped_by) in [
        (vec!["--fail-fast"], 2, "fail_fast"),
        (vec!["--max-errors", "2"], 4, "max_errors"),
    ] {
        let server = mock(true).await;
        let mut command = jev(Some(&server));
        command.args([
            "batch",
            "run",
            "-f",
            &questions,
            "--input",
            &input,
            "--state-field",
            "text",
        ]);
        command.args(["--concurrency", "1"]).args(&flags);

        let run = run(command).await;

        assert_eq!(run.code, 7, "{flags:?}: {}", run.stderr);
        assert_eq!(lines_of(&run.stdout).len(), sent_rows, "{flags:?}");
        assert_eq!(sent(&server).await.len(), sent_rows, "{flags:?}");
        let summary = &json_of(&run.stderr)["summary"];
        assert_eq!(summary["rows_total"], 10);
        assert_eq!(summary["skipped"], 10 - sent_rows);
        assert_eq!(summary["stopped_by"], stopped_by);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repeated_id_or_a_missing_field_fails_before_any_request() {
    let server = mock(true).await;
    let dir = scratch("preflight");
    let questions = write(&dir, "questions.yaml", QUESTIONS);
    let repeated = write(
        &dir,
        "repeated.jsonl",
        "{\"id\": \"a\", \"text\": \"x\"}\n{\"id\": \"b\", \"text\": \"y\"}\n{\"id\": \"a\", \"text\": \"z\"}\n",
    );
    let missing = write(
        &dir,
        "missing.jsonl",
        "{\"text\": \"x\"}\n{\"body\": \"SENTINEL-STATE\"}\n",
    );
    let csv = write(&dir, "tickets.csv", "id,subject\n1,Refund\n");

    let cases = [
        (
            vec!["--input", &repeated, "--id-field", "id"],
            "line 3: the id `a` is used by an earlier row; ids must be unique",
        ),
        (
            vec!["--input", &missing, "--state-field", "text"],
            "line 2: the row has no field `text`",
        ),
        (vec!["--input", &csv, "--state-field", "body"], ""),
    ];
    for (flags, problem) in cases {
        let mut command = jev(Some(&server));
        command
            .args(["batch", "run", "-f", &questions])
            .args(&flags);

        let run = run(command).await;

        assert_eq!(
            (run.code, run.stdout.as_str()),
            (2, ""),
            "{flags:?}: {}",
            run.stderr
        );
        let error = &json_of(&run.stderr)["error"];
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .ends_with("nothing was sent"),
            "{error}"
        );
        if !problem.is_empty() {
            assert_eq!(
                error["details"]["problems"][0]["message"],
                problem.split_once(": ").unwrap().1
            );
        }
        assert!(
            !run.stderr.contains("SENTINEL-STATE"),
            "a row is never quoted"
        );
    }
    assert!(sent(&server).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn csv_rows_send_a_trimmed_state_and_carry_their_own_ids() {
    let server = mock(true).await;
    let dir = scratch("csv");
    let questions = write(&dir, "questions.yaml", QUESTIONS);
    let input = write(
        &dir,
        "tickets.csv",
        "ticket,subject,body,internal\nT1,Refund,\"Please, now\",secret\nT2,Login,Cannot sign in,secret\n",
    );
    let mut command = jev(Some(&server));
    command.args(["batch", "run", "-f", &questions, "--input", &input]);
    command.args(["--state-fields", "subject,body", "--id-field", "ticket"]);

    let run = run(command).await;

    assert_eq!(run.code, 0, "{}", run.stderr);
    let ids: BTreeSet<String> = lines_of(&run.stdout)
        .iter()
        .map(|record| record["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, BTreeSet::from(["T1".to_owned(), "T2".to_owned()]));
    let states: BTreeSet<String> = sent(&server)
        .await
        .iter()
        .map(|body| body["state"].to_string())
        .collect();
    assert_eq!(
        states,
        BTreeSet::from([
            r#"{"subject":"Login","body":"Cannot sign in"}"#.to_owned(),
            r#"{"subject":"Refund","body":"Please, now"}"#.to_owned(),
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn piped_rows_are_read_from_stdin_and_identified_by_their_line() {
    let server = mock(true).await;
    let dir = scratch("stdin");
    let questions = write(&dir, "questions.yaml", QUESTIONS);
    let mut command = jev(Some(&server));
    command.args(["batch", "run", "-f", &questions, "-o", "table"]);
    command.write_stdin("\"first\"\n\n\"third\"\n");

    let run = run(command).await;

    assert_eq!(run.code, 0, "{}", run.stderr);
    let mut ids: Vec<u64> = lines_of(&run.stdout)
        .iter()
        .map(|record| record["id"].as_u64().unwrap())
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, [1, 3], "records are JSONL whatever --output says");
    assert!(
        run.stderr
            .starts_with("batch: 2 rows: 2 ok, 0 failed, 0 skipped\n"),
        "a person gets a text summary: {}",
        run.stderr
    );
    let states: BTreeSet<String> = sent(&server)
        .await
        .iter()
        .map(|body| body["state"].to_string())
        .collect();
    assert_eq!(
        states,
        BTreeSet::from(["\"first\"".to_owned(), "\"third\"".to_owned()])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_is_sent_for_a_bad_question_set_a_used_output_file_or_a_missing_key() {
    let server = mock(true).await;
    let dir = scratch("refusals");
    let questions = write(&dir, "questions.yaml", QUESTIONS);
    let invalid = write(
        &dir,
        "invalid.yaml",
        "questions:\n  urgent:\n    type: noul\n    instructions: 42\n",
    );
    let input = write(&dir, "rows.jsonl", &rows(3, &[]));
    let used = write(&dir, "used.jsonl", "{\"id\": 1}\n");
    let fresh = dir.join("fresh.jsonl");

    let bad_questions = run({
        let mut command = jev(Some(&server));
        command.args(["batch", "run", "-f", &invalid, "--input", &input]);
        command
    })
    .await;
    let used_output = run({
        let mut command = jev(Some(&server));
        command.args([
            "batch", "run", "-f", &questions, "--input", &input, "--out", &used,
        ]);
        command
    })
    .await;
    let no_key = run({
        let mut command = jev(Some(&server));
        command.env_remove("TYPESAFE_API_KEY");
        command
            .args(["batch", "run", "-f", &questions, "--input", &input, "--out"])
            .arg(&fresh);
        command
    })
    .await;
    let field = run({
        let mut command = jev(Some(&server));
        command.args([
            "batch", "run", "-f", &questions, "--input", &input, "--field", "id",
        ]);
        command
    })
    .await;

    assert_eq!(bad_questions.code, 2, "{}", bad_questions.stderr);
    assert!(
        json_of(&bad_questions.stderr)["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("the request is not valid")
    );
    assert_eq!(used_output.code, 2);
    assert_eq!(
        json_of(&used_output.stderr)["error"]["message"],
        format!("the output file {used} already holds records")
    );
    assert_eq!(
        fs::read_to_string(&used).unwrap(),
        "{\"id\": 1}\n",
        "never overwritten"
    );
    assert_eq!(no_key.code, 3, "{}", no_key.stderr);
    assert!(!fresh.exists(), "no output file is left behind");
    assert_eq!(field.code, 2);
    assert!(sent(&server).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_concurrency_above_eight_is_allowed_with_a_warning_and_above_64_refused() {
    let server = mock(true).await;
    let dir = scratch("concurrency");
    let questions = write(&dir, "questions.yaml", QUESTIONS);
    let input = write(&dir, "rows.jsonl", &rows(2, &[]));

    let high = run({
        let mut command = jev(Some(&server));
        command.args([
            "batch",
            "run",
            "-f",
            &questions,
            "--input",
            &input,
            "--concurrency",
            "12",
        ]);
        command
    })
    .await;
    let too_high = run({
        let mut command = jev(Some(&server));
        command.args([
            "batch",
            "run",
            "-f",
            &questions,
            "--input",
            &input,
            "--concurrency",
            "65",
        ]);
        command
    })
    .await;

    assert_eq!(high.code, 0, "{}", high.stderr);
    let stderr = lines_of(&high.stderr);
    assert_eq!(stderr[0]["warning"]["code"], "high_concurrency");
    assert_eq!(stderr[1]["summary"]["ok"], 2);
    assert_eq!((too_high.code, too_high.stdout.as_str()), (2, ""));
}

/// NFR-PERF-3: memory does not grow with the input. Run with
/// `cargo test --release -p jev-cli --test batch -- --ignored --nocapture`.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "a benchmark: a million requests take a few minutes"]
async fn a_million_rows_stay_under_100_mb_of_memory() {
    let server = mock(false).await;
    let dir = scratch("million");
    let questions = write(&dir, "questions.yaml", QUESTIONS);
    let input = dir.join("rows.jsonl");
    {
        use std::io::Write as _;
        let mut file = std::io::BufWriter::new(fs::File::create(&input).unwrap());
        for n in 1..=1_000_000_u64 {
            writeln!(file, "{{\"id\": \"ticket-{n}\", \"text\": \"The payout for order {n} failed twice today.\"}}").unwrap();
        }
    }
    let out = dir.join("results.jsonl");
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("jev"))
        .args([
            "batch",
            "run",
            "-f",
            &questions,
            "--state-field",
            "text",
            "--id-field",
            "id",
        ])
        .args(["--concurrency", "16", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .env("JEV_CONFIG_DIR", dir.join("config"))
        .env("TYPESAFE_API_KEY", SENTINEL_KEY)
        .env("TYPESAFE_BASE_URL", server.uri())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let status = format!("/proc/{}/status", child.id());
    let mut peak_kb = 0_u64;
    while child.try_wait().unwrap().is_none() {
        if let Some(kb) = fs::read_to_string(&status).ok().and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("VmHWM:"))
                .and_then(|value| value.trim().trim_end_matches("kB").trim().parse().ok())
        }) {
            peak_kb = peak_kb.max(kb);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let output = child.wait_with_output().unwrap();

    eprintln!("peak RSS {} MB", peak_kb / 1024);
    eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success());
    assert!(peak_kb < 100 * 1024, "peak RSS {peak_kb} kB");
}
