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

#[cfg(unix)]
#[path = "support/pty.rs"]
mod pty;

const SENTINEL_KEY: &str = "sentinel-key-do-not-leak-7a1e";

/// `jev`, isolated from the environment of whoever runs the tests, pointed at `server`.
fn jev(server: Option<&MockServer>) -> Command {
    Command::from_std(process(server))
}

/// The same, as a process to start and stop by hand.
fn process(server: Option<&MockServer>) -> std::process::Command {
    let mut command = std::process::Command::new(assert_cmd::cargo::cargo_bin("jev"));
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
    // Piped, so the summary on stderr is one JSON line, the same as the file, after any progress
    // lines a slow machine took long enough to get.
    let stderr = lines_of(&run.stderr);
    let (last, before) = stderr.split_last().unwrap();
    assert_eq!(last["summary"]["ok"], 10_000, "{}", run.stderr);
    assert!(
        before.iter().all(|line| line.get("progress").is_some()),
        "{}",
        run.stderr
    );
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

/// Starts `jev batch run` in the background, writing to `out`.
fn start(server: &MockServer, arguments: &[&str]) -> std::process::Child {
    process(Some(server))
        .args(["batch", "run"])
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap()
}

/// Waits until `out` holds at least `count` complete records.
async fn wait_for_records(out: &Path, count: usize) {
    for _ in 0..600 {
        let written = fs::read_to_string(out).unwrap_or_default();
        if written.matches('\n').count() >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{} never held {count} records", out.display());
}

/// The id of the row each request was sent for: row n's state is `fine n`.
async fn sent_ids(server: &MockServer) -> Vec<u64> {
    sent(server)
        .await
        .iter()
        .map(|body| {
            let state = body["state"].as_str().unwrap();
            state.rsplit_once(' ').unwrap().1.parse().unwrap()
        })
        .collect()
}

/// A mock that answers every row after `delay`.
async fn slow_mock(delay: Duration) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(delay)
                .set_body_json(answer()),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test(flavor = "multi_thread")]
async fn a_killed_run_resumed_never_sends_a_finished_row_again() {
    let dir = scratch("resume-after-kill");
    let questions = write(&dir, "questions.yaml", QUESTIONS);
    let input = write(&dir, "rows.jsonl", &rows(40, &[]));
    let out = dir.join("results.jsonl");
    let out_path = out.to_str().unwrap();
    let arguments = [
        "-f",
        &questions,
        "--input",
        &input,
        "--state-field",
        "text",
        "--concurrency",
        "2",
        "--out",
        out_path,
    ];
    let first = slow_mock(Duration::from_millis(50)).await;
    let mut child = start(&first, &arguments);
    wait_for_records(&out, 6).await;
    child.kill().unwrap();
    child.wait().unwrap();
    let done: BTreeSet<u64> = lines_of(&fs::read_to_string(&out).unwrap())
        .iter()
        .map(|record| record["id"].as_u64().unwrap())
        .collect();
    assert!(done.len() >= 6 && done.len() < 40, "{done:?}");

    let second = mock(true).await;
    let resumed = run({
        let mut command = jev(Some(&second));
        command.args(["batch", "run", "--resume"]).args(arguments);
        command
    })
    .await;

    assert_eq!(resumed.code, 0, "{}", resumed.stderr);
    let resent: BTreeSet<u64> = sent_ids(&second).await.into_iter().collect();
    assert!(resent.is_disjoint(&done), "finished rows were sent again");
    assert_eq!(resent.len() + done.len(), 40);
    let records = lines_of(&fs::read_to_string(&out).unwrap());
    let mut ok: Vec<u64> = records
        .iter()
        .filter(|record| record["status"] == "ok")
        .map(|record| record["id"].as_u64().unwrap())
        .collect();
    ok.sort_unstable();
    assert_eq!(
        ok,
        (1..=40).collect::<Vec<_>>(),
        "exactly one ok record per id"
    );
    let summary = &json_of(&resumed.stderr)["summary"];
    assert_eq!(summary["already_ok"], done.len());
    assert_eq!(summary["ok"], 40 - done.len());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cut_off_last_line_is_discarded_and_failed_rows_are_retried_on_resume() {
    let server = mock(true).await;
    let dir = scratch("resume-truncated");
    let questions = write(&dir, "questions.yaml", QUESTIONS);
    let input = write(&dir, "rows.jsonl", &rows(5, &[]));
    let out = write(
        &dir,
        "results.jsonl",
        "{\"id\":1,\"status\":\"ok\"}\n{\"id\":2,\"status\":\"error\",\"error\":{}}\n{\"id\":3,\"status\":\"ok\"}\n{\"id\":4,\"status\":\"o",
    );
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
        "--out",
        &out,
        "--resume",
    ]);

    let run = run(command).await;

    assert_eq!(run.code, 0, "{}", run.stderr);
    let mut resent = sent_ids(&server).await;
    resent.sort_unstable();
    assert_eq!(resent, [2, 4, 5]);
    let records = lines_of(&fs::read_to_string(&out).unwrap());
    assert_eq!(records.len(), 6, "every line is a whole record");
    let stderr = lines_of(&run.stderr);
    assert_eq!(stderr[0]["warning"]["code"], "partial_record_discarded");
    assert_eq!(
        (
            &stderr[1]["summary"]["ok"],
            &stderr[1]["summary"]["already_ok"]
        ),
        (&json!(3), &json!(2))
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn sigint_finishes_the_rows_in_flight_prints_the_summary_and_exits_130() {
    let server = slow_mock(Duration::from_millis(300)).await;
    let dir = scratch("sigint");
    let questions = write(&dir, "questions.yaml", QUESTIONS);
    let input = write(&dir, "rows.jsonl", &rows(40, &[]));
    let out = dir.join("results.jsonl");
    let child = start(
        &server,
        &[
            "-f",
            &questions,
            "--input",
            &input,
            "--state-field",
            "text",
            "--concurrency",
            "2",
            "--out",
            out.to_str().unwrap(),
        ],
    );
    wait_for_records(&out, 2).await;
    let status = std::process::Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let output = tokio::task::spawn_blocking(move || child.wait_with_output().unwrap())
        .await
        .unwrap();

    assert_eq!(output.status.code(), Some(130));
    assert!(output.stdout.is_empty());
    let records = lines_of(&fs::read_to_string(&out).unwrap());
    assert!(
        records.len() >= 2 && records.len() < 40,
        "{}",
        records.len()
    );
    assert!(records.iter().all(|record| record["status"] == "ok"));
    let stderr = lines_of(&String::from_utf8(output.stderr).unwrap());
    assert_eq!(stderr[0]["warning"]["code"], "interrupted");
    let summary = &stderr[1]["summary"];
    assert_eq!(summary["stopped_by"], "interrupted");
    assert_eq!(
        summary["ok"],
        records.len(),
        "the rows in flight were recorded"
    );
    assert_eq!(summary["skipped"], 40 - records.len());
    assert_eq!(
        sent(&server).await.len(),
        records.len(),
        "nothing sent after"
    );
}

/// Answers 429 with `retry-after: 1` to the first request, then 200 after 200 ms, and remembers
/// when each request arrived. The 200s are slow so that a worker whose first request was already
/// on its way cannot finish it and start another before the refused worker has seen its 429.
struct RateLimited {
    arrivals: std::sync::Mutex<Vec<std::time::Instant>>,
}

impl wiremock::Respond for RateLimited {
    fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
        let mut arrivals = self.arrivals.lock().unwrap();
        arrivals.push(std::time::Instant::now());
        if arrivals.len() == 1 {
            ResponseTemplate::new(429)
                .insert_header("retry-after", "1")
                .set_body_json(json!({ "detail": { "error_type": "rate_limit_error", "message": "Slow down." } }))
        } else {
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(200))
                .set_body_json(answer())
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_429_on_one_worker_pauses_every_worker_for_its_retry_after() {
    let server = MockServer::start().await;
    let arrivals = std::sync::Arc::new(RateLimited {
        arrivals: std::sync::Mutex::new(Vec::new()),
    });
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(Shared(std::sync::Arc::clone(&arrivals)))
        .mount(&server)
        .await;
    let dir = scratch("rate-limited");
    let questions = write(&dir, "questions.yaml", QUESTIONS);
    let input = write(&dir, "rows.jsonl", &rows(12, &[]));
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
        "--concurrency",
        "4",
    ]);

    let run = run(command).await;

    assert_eq!(run.code, 0, "{}", run.stderr);
    let records = lines_of(&run.stdout);
    assert_eq!(records.len(), 12);
    assert!(
        records.iter().all(|record| record["status"] == "ok"),
        "no 429 went unrecovered"
    );
    let arrivals = arrivals.arrivals.lock().unwrap().clone();
    assert_eq!(arrivals.len(), 13, "one retry");
    let refused = arrivals[0];
    let mut since: Vec<Duration> = arrivals
        .iter()
        .map(|arrival| arrival.duration_since(refused))
        .collect();
    since.sort_unstable();
    // The bounds below hold however slow the machine, because sleeps never end early; they
    // assume only that the refused worker sees its 429 before another worker's 200 arrives.
    //
    // During the server's second, at most the other three workers' first requests arrive: those
    // that had passed the throttle before the 429 came back. How many had depends on how fast the
    // workers started, so it is a bound, not a count. A throttle that paused only the refused
    // worker would let the others run through every row in that second.
    let pause = Duration::from_millis(990);
    let (during, after) = since.split_at(since.partition_point(|since| *since < pause));
    assert!(during.len() <= 4, "{since:?}");
    // After it, calls start one at a time. The gap starts at 100 ms and narrows by an eighth with
    // each success, so with at most 11 successes before the last start it stays above 20 ms: the
    // n-th paced start is at least n - 1 gaps after the pause. Of the arrivals after the pause,
    // all are paced but the first requests that were on their way yet arrived late, of which
    // there are at most `3 - early`.
    let early = during.len() - 1;
    for (index, arrival) in after.iter().enumerate() {
        let paced_before = (index + early).saturating_sub(3);
        let gaps = u32::try_from(paced_before).unwrap();
        assert!(
            *arrival >= pause + Duration::from_millis(20) * gaps,
            "{since:?}"
        );
    }
    assert_eq!(json_of(&run.stderr)["summary"]["retries"], 1);
}

/// Lets a test keep a handle on a responder it mounts.
struct Shared<T>(std::sync::Arc<T>);

impl<T: wiremock::Respond> wiremock::Respond for Shared<T> {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        self.0.respond(request)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_counts_and_prices_the_requests_without_a_key_or_a_request() {
    let server = mock(true).await;
    let dir = scratch("dry-run");
    let questions = write(&dir, "questions.yaml", QUESTIONS);
    let input = write(&dir, "rows.jsonl", &rows(10, &[]));
    let out = write(
        &dir,
        "results.jsonl",
        "{\"id\":1,\"status\":\"ok\"}\n{\"id\":2,\"status\":\"ok\"}\n",
    );

    let planned = run({
        let mut command = jev(Some(&server));
        command.env_remove("TYPESAFE_API_KEY").args([
            "batch",
            "run",
            "-f",
            &questions,
            "--input",
            &input,
            "--state-field",
            "text",
            "--model",
            "jev-1.13.0",
            "--out",
            &out,
            "--resume",
            "--limit",
            "8",
            "--dry-run",
        ]);
        command
    })
    .await;
    let field = run({
        let mut command = jev(Some(&server));
        command.args([
            "batch",
            "run",
            "-f",
            &questions,
            "--input",
            &input,
            "--dry-run",
            "--field",
            "requests",
        ]);
        command
    })
    .await;

    assert_eq!(planned.code, 0, "{}", planned.stderr);
    let plan = json_of(&planned.stdout);
    assert_eq!(plan["dry_run"], true);
    assert_eq!(
        (
            &plan["rows_total"],
            &plan["requests"],
            &plan["already_ok"],
            &plan["invalid_rows"]
        ),
        (&json!(8), &json!(6), &json!(2), &json!(0))
    );
    let tokens = plan["estimated_input_tokens"].as_u64().unwrap();
    assert!(tokens > 6 * 10, "{plan}");
    #[allow(clippy::cast_precision_loss)]
    let expected = tokens as f64 * 0.042 / 1e6;
    assert!(
        (plan["estimated_cost_usd"].as_f64().unwrap() - expected).abs() < 1e-9,
        "{plan}"
    );
    assert_eq!(plan["requested_model"], "jev-1.13.0");
    assert_eq!(
        fs::read_to_string(&out).unwrap().lines().count(),
        2,
        "untouched"
    );
    assert_eq!(
        (field.code, field.stdout.trim()),
        (0, "10"),
        "{}",
        field.stderr
    );
    assert!(sent(&server).await.is_empty(), "a dry run sends nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_lists_the_rows_that_cannot_be_used_and_exits_2() {
    let server = mock(true).await;
    let dir = scratch("dry-run-invalid");
    let questions = write(&dir, "questions.yaml", QUESTIONS);
    let bad = write(
        &dir,
        "bad.jsonl",
        "{\"text\": \"fine\"}\n{\"body\": \"SENTINEL-STATE\"}\n",
    );

    let invalid = run({
        let mut command = jev(Some(&server));
        command.args([
            "batch",
            "run",
            "-f",
            &questions,
            "--input",
            &bad,
            "--state-field",
            "text",
            "--dry-run",
        ]);
        command
    })
    .await;

    assert_eq!(invalid.code, 2);
    let problems = &json_of(&invalid.stdout)["problems"];
    assert_eq!(
        problems,
        &json!([{ "line": 2, "message": "the row has no field `text`" }])
    );
    assert!(
        json_of(&invalid.stderr)["error"]["message"]
            .as_str()
            .unwrap()
            .ends_with("nothing was sent")
    );
    assert!(
        !invalid.stdout.contains("SENTINEL-STATE") && !invalid.stderr.contains("SENTINEL-STATE")
    );
    assert!(sent(&server).await.is_empty(), "a dry run sends nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn ordered_records_follow_the_input_when_the_first_row_is_the_slowest() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(body_string_contains("fine 1\""))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(300))
                .set_body_json(answer()),
        )
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(answer()))
        .mount(&server)
        .await;
    let dir = scratch("ordered");
    let questions = write(&dir, "questions.yaml", QUESTIONS);
    let input = write(&dir, "rows.jsonl", &rows(8, &[]));
    let arguments = [
        "batch",
        "run",
        "-f",
        &questions,
        "--input",
        &input,
        "--state-field",
        "text",
    ];

    let ordered = run({
        let mut command = jev(Some(&server));
        command.args(arguments).arg("--ordered");
        command
    })
    .await;
    let unordered = run({
        let mut command = jev(Some(&server));
        command.args(arguments);
        command
    })
    .await;

    let ids = |stdout: &str| -> Vec<u64> {
        lines_of(stdout)
            .iter()
            .map(|record| record["id"].as_u64().unwrap())
            .collect()
    };
    assert_eq!(ordered.code, 0, "{}", ordered.stderr);
    assert_eq!(ids(&ordered.stdout), (1..=8).collect::<Vec<_>>());
    assert_eq!(
        ids(&unordered.stdout).last(),
        Some(&1),
        "without --ordered, the slow row is last"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_gets_a_progress_bar_that_is_erased_before_the_summary() {
    let server = slow_mock(Duration::from_millis(20)).await;
    let dir = scratch("progress-bar");
    let questions = write(&dir, "questions.yaml", QUESTIONS);
    let input = write(&dir, "rows.jsonl", &rows(20, &[]));
    let out = dir.join("results.jsonl");
    let mut command = process(Some(&server));
    command
        .args([
            "batch",
            "run",
            "-f",
            &questions,
            "--input",
            &input,
            "--state-field",
            "text",
        ])
        .args(["--concurrency", "2", "--out"])
        .arg(&out)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .env("TERM", "xterm-256color")
        .env("NO_COLOR", "1")
        .env("JEV_CONFIG_DIR", dir.join("config"));

    let (status, seen) =
        tokio::task::spawn_blocking(move || pty::run(command, pty::Stream::Stderr))
            .await
            .unwrap();

    assert!(status.success(), "{status}: {seen:?}");
    assert!(!seen.contains(SENTINEL_KEY));
    let bar = "\r\u{1b}[2K";
    assert!(seen.contains(&format!("{bar}#")), "{seen:?}");
    assert!(seen.contains("/20 rows | "), "{seen:?}");
    assert!(
        seen.contains(&format!(
            "{bar}{{\"summary\":{{\"rows_total\":20,\"ok\":20,"
        )),
        "the bar is erased before the summary: {seen:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pipe_gets_a_progress_line_every_few_seconds_unless_quiet() {
    // Twelve rows one at a time, half a second each: past the 5 s between lines.
    let server = slow_mock(Duration::from_millis(500)).await;
    let dir = scratch("progress-lines");
    let questions = write(&dir, "questions.yaml", QUESTIONS);
    let input = write(&dir, "rows.jsonl", &rows(12, &[]));
    let command = |quiet: bool| {
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
            "--concurrency",
            "1",
        ]);
        if quiet {
            command.arg("--quiet");
        }
        command
    };

    let (default, quiet) = tokio::join!(run(command(false)), run(command(true)));

    for run in [&default, &quiet] {
        assert_eq!(run.code, 0, "{}", run.stderr);
        let records = lines_of(&run.stdout);
        assert_eq!(records.len(), 12, "stdout is the records and nothing else");
        assert!(records.iter().all(|record| record["status"] == "ok"));
    }
    let progress: Vec<Value> = lines_of(&default.stderr)
        .into_iter()
        .filter(|line| line.get("progress").is_some())
        .collect();
    assert!(!progress.is_empty(), "{}", default.stderr);
    assert_eq!(progress[0]["progress"]["rows_total"], 12);
    assert!(progress[0]["progress"]["elapsed_ms"].as_u64().unwrap() >= 5000);
    assert!(!quiet.stderr.contains("progress"), "{}", quiet.stderr);
}
