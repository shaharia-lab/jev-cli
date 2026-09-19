//! Performance budgets (PRD NFR-PERF-1, NFR-PERF-2), timed on the built `jev` binary.
//!
//! These are benchmarks, so they are ignored by default: a debug build, or a machine busy running
//! other tests, says nothing about them. Run them on a release build, one at a time:
//!
//! ```text
//! cargo test --release -p jev-cli --test performance -- --ignored --nocapture --test-threads=1
//! ```
//!
//! CI runs exactly that on Linux. Each test runs `jev` a fixed number of times after one warm-up
//! run, prints the median and the 95th percentile, and checks the median against two limits: the
//! PRD's budget, and a much tighter regression guard of about two and a half times what CI
//! measured when the guard was set, so that a change which doubles start-up fails there. The
//! median rather than the 95th percentile (on which the PRD states its budgets) is checked so
//! that one slow run on a shared CI machine cannot fail the build. The guards are set for that
//! runner: on a slower machine only the budgets say something.
//!
//! NFR-PERF-3 (batch memory is proportional to the concurrency, not to the input) is
//! `a_million_rows_stay_under_100_mb_of_memory` in `tests/batch.rs`.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// A key that must never appear in any output. It is not a real credential.
const SENTINEL_KEY: &str = "sentinel-key-do-not-leak-41c7";

/// How many timed runs each measurement takes, after one warm-up run.
const RUNS: usize = 30;

/// NFR-PERF-1: process start to request sent, plus response received to exit.
const OVERHEAD_BUDGET: Duration = Duration::from_millis(30);

/// NFR-PERF-2: a command that needs no network.
const OFFLINE_BUDGET: Duration = Duration::from_millis(50);

/// The regression guards, about two and a half times the medians CI's Linux runner measured when
/// they were set (in brackets). One that trips means start-up has roughly doubled: find out why
/// before raising it, and never above the budget.
const HELP_GUARD: Duration = Duration::from_millis(5); // (1.8 ms)
const SPEC_GUARD: Duration = Duration::from_millis(7); // (2.8 ms)
const VALIDATE_GUARD: Duration = Duration::from_millis(5); // (1.8 ms)
const OVERHEAD_GUARD: Duration = Duration::from_millis(18); // (7.4 ms)

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// An address nothing listens on, so that a benchmark cannot reach the real API.
const NOWHERE: &str = "http://127.0.0.1:9";

/// `jev`, isolated from the environment of whoever runs the benchmark, pointed at `base_url`.
fn jev(base_url: &str, arguments: &[&str]) -> Command {
    let mut command = Command::new(assert_cmd::cargo::cargo_bin("jev"));
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
    command
        .args(arguments)
        .env(
            "JEV_CONFIG_DIR",
            Path::new(env!("CARGO_TARGET_TMPDIR")).join("no-config"),
        )
        .env("TYPESAFE_API_KEY", SENTINEL_KEY)
        .env("TYPESAFE_BASE_URL", base_url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// Runs `command` to completion and returns when it started and when it ended.
fn time(command: &mut Command) -> (Instant, Instant) {
    let start = Instant::now();
    let status = command.status().unwrap();
    let end = Instant::now();
    assert!(status.success(), "{command:?} failed: {status}");
    (start, end)
}

/// The median and the 95th percentile (nearest rank) of some timings.
fn summarise(mut timings: Vec<Duration>) -> (Duration, Duration) {
    timings.sort_unstable();
    // Nearest rank: the smallest value with at least `percent` of them at or below it.
    let rank = |percent: usize| {
        let index = (timings.len() * percent).div_ceil(100).saturating_sub(1);
        *timings.get(index).unwrap()
    };
    (rank(50), rank(95))
}

/// Prints a measurement and fails when its median is over the budget or the guard.
fn check(what: &str, timings: Vec<Duration>, budget: Duration, guard: Duration) {
    let (median, p95) = summarise(timings);
    eprintln!(
        "{what}: median {:.1} ms, p95 {:.1} ms (budget {} ms, guard {} ms)",
        median.as_secs_f64() * 1000.0,
        p95.as_secs_f64() * 1000.0,
        budget.as_millis(),
        guard.as_millis()
    );
    assert!(
        median <= budget,
        "{what} takes {median:?} (median of {RUNS} runs), over the budget of {budget:?}"
    );
    assert!(
        median <= guard,
        "{what} takes {median:?} (median of {RUNS} runs), over the regression guard of {guard:?}: start-up has become much slower"
    );
}

/// NFR-PERF-2: help, the machine-readable spec and offline validation are quick.
#[test]
#[ignore = "a benchmark: run it on a release build, as the module documentation says"]
fn commands_that_need_no_network_finish_within_50_ms() {
    let request = fixture("eval/triage.json");
    let request = request.to_str().unwrap();
    for (arguments, guard) in [
        (vec!["--help"], HELP_GUARD),
        (vec!["spec"], SPEC_GUARD),
        (vec!["validate", "-f", request], VALIDATE_GUARD),
    ] {
        time(&mut jev(NOWHERE, &arguments));
        let timings = (0..RUNS)
            .map(|_| {
                let (start, end) = time(&mut jev(NOWHERE, &arguments));
                end - start
            })
            .collect();
        check(
            &format!("jev {}", arguments.join(" ")),
            timings,
            OFFLINE_BUDGET,
            guard,
        );
    }
}

/// Answers every request with `response.json`, and notes when each one arrived.
struct Recorder {
    arrivals: Arc<Mutex<Vec<Instant>>>,
    body: serde_json::Value,
}

impl Respond for Recorder {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.arrivals.lock().unwrap().push(Instant::now());
        ResponseTemplate::new(200)
            .insert_header("x-typesafe-request-id", "req_benchmark")
            .set_body_json(&self.body)
    }
}

/// NFR-PERF-1: `jev eval` adds little to the API's own latency. The mock answers at once, so
/// what is measured is jev's own time: from starting the process to the request arriving, and
/// from the answer leaving to the process ending (which includes the loopback round trip).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "a benchmark: run it on a release build, as the module documentation says"]
async fn eval_adds_under_30_ms_to_a_request() {
    let server = MockServer::start().await;
    let arrivals = Arc::new(Mutex::new(Vec::new()));
    let body = std::fs::read_to_string(fixture("eval/response.json")).unwrap();
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(Recorder {
            arrivals: Arc::clone(&arrivals),
            body: serde_json::from_str(&body).unwrap(),
        })
        .mount(&server)
        .await;
    let request = fixture("eval/triage.json");
    let base_url = server.uri();

    // Off the async runtime's threads, since the mock server lives on them.
    let runs = tokio::task::spawn_blocking(move || {
        let request = request.to_str().unwrap();
        let run = || time(&mut jev(&base_url, &["eval", "-f", request, "-o", "json"]));
        run();
        (0..RUNS).map(|_| run()).collect::<Vec<_>>()
    })
    .await
    .unwrap();

    let arrivals = arrivals.lock().unwrap().clone();
    assert_eq!(arrivals.len(), RUNS + 1, "one request per run");
    let (mut before, mut after, mut total) = (Vec::new(), Vec::new(), Vec::new());
    for ((start, end), arrived) in runs.into_iter().zip(arrivals.into_iter().skip(1)) {
        before.push(arrived - start);
        after.push(end - arrived);
        total.push(end - start);
    }
    // Printed apart so that a regression points at its half; the budget is on the two together.
    for (what, timings) in [("start to request sent", before), ("answer to exit", after)] {
        let (median, p95) = summarise(timings);
        eprintln!(
            "jev eval, {what}: median {:.1} ms, p95 {:.1} ms",
            median.as_secs_f64() * 1000.0,
            p95.as_secs_f64() * 1000.0
        );
    }
    check(
        "jev eval, overhead in all",
        total,
        OVERHEAD_BUDGET,
        OVERHEAD_GUARD,
    );
}
