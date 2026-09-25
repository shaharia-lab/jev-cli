//! `jev noul`, `jev choice`, `jev score` and the gates, against a local mock of the API.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::Path;
use std::time::Duration;

use assert_cmd::Command;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
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

/// A mock that answers the one question of a shortcut with `answer`.
async fn answering(answer: Value) -> MockServer {
    let server = MockServer::start().await;
    let body = json!({ "model": "jev-1.13.0", "answers": { "answer": answer }, "usage": { "input_tokens": 300, "output_tokens": 20 } });
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-typesafe-request-id", "req_ok")
                .set_body_json(body),
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

fn noul(probability: f64) -> Value {
    json!({ "type": "noul", "noul": probability })
}

fn choice(winner: &str, confidence: f64) -> Value {
    json!({ "type": "choice", "choice": winner, "confidence": confidence, "probabilities": { "billing": 0.5, "sales": 0.3, "other": 0.2 } })
}

fn score(value: f64, confidence: f64) -> Value {
    json!({ "type": "score", "score": value, "confidence": confidence, "legend": { "0": "Calm", "1": "Angry" }, "probabilities": { "0": 0.4, "1": 0.6 } })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_noul_gate_exits_0_when_it_holds_and_10_when_it_does_not_printing_the_answer_both_times()
{
    for (probability, exit, passed) in [(0.92, 0, true), (0.31, 10, false)] {
        let server = answering(noul(probability)).await;
        let mut command = jev(Some(&server));
        command.args([
            "noul",
            "Does this convey urgency?",
            "--state",
            "Payouts are failing!",
            "--fail-under",
            "0.7",
        ]);

        let run = run(command).await;

        assert_eq!(run.code, exit, "{probability}");
        assert_eq!(
            run.stderr, "",
            "a gate that is false is not an error, so stderr stays empty"
        );
        let result = json_of(&run.stdout);
        assert_eq!(
            result["noul"], probability,
            "the answer is printed either way"
        );
        assert_eq!(result["gate"]["passed"], passed);
        assert_eq!(
            result["gate"]["conditions"][0],
            json!({ "condition": "answer >= 0.7", "passed": passed, "actual": probability })
        );
        assert_eq!(result["model"], "jev-1.13.0");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_shortcut_builds_the_same_request_a_file_would() {
    let server = answering(noul(0.9)).await;
    let mut command = jev(Some(&server));
    command
        .args([
            "noul",
            "Does this convey urgency?",
            "--true",
            "Time-sensitive",
            "--false",
            "No urgency",
            "--model",
            "jev-1.13.0",
        ])
        .write_stdin("Payouts are failing!\n");

    assert_eq!(run(command).await.code, 0);

    assert_eq!(
        sent(&server).await[0],
        json!({
            "questions": { "answer": {
                "type": "noul",
                "instructions": "Does this convey urgency?",
                "criteria": { "true": "Time-sensitive", "false": "No urgency" }
            }},
            "model": "jev-1.13.0",
            "state": "Payouts are failing!\n"
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fail_over_and_the_abstain_band() {
    let cases: [(f64, Vec<&str>, i32); 5] = [
        (0.2, vec!["--fail-over", "0.3"], 0),
        (0.4, vec!["--fail-over", "0.3"], 10),
        (
            0.5,
            vec!["--fail-under", "0.7", "--abstain-band", "0.4,0.6"],
            11,
        ),
        (0.5, vec!["--abstain-band", "0.4,0.6"], 11),
        (
            0.9,
            vec!["--fail-under", "0.7", "--abstain-band", "0.4,0.6"],
            0,
        ),
    ];
    for (probability, flags, exit) in cases {
        let server = answering(noul(probability)).await;
        let mut command = jev(Some(&server));
        command
            .args(["noul", "Is it spam?", "--state", "s"])
            .args(&flags);

        let run = run(command).await;

        assert_eq!(run.code, exit, "{probability} {flags:?}");
        assert_eq!(json_of(&run.stdout)["gate"]["abstained"], exit == 11);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failure_with_a_gate_present_is_never_exit_10() {
    // Nothing listens here: a network failure, with a gate on the command line.
    let mut network = jev(None);
    network.args([
        "noul",
        "Is it urgent?",
        "--state",
        "s",
        "--fail-under",
        "0.7",
        "--max-retries",
        "0",
        "--timeout",
        "2",
    ]);
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(401).set_body_json(json!({ "detail": { "error_type": "authentication_error", "message": "Invalid API key." } }))).mount(&server).await;
    let mut auth = jev(Some(&server));
    auth.args([
        "noul",
        "Is it urgent?",
        "--state",
        "s",
        "--fail-under",
        "0.7",
    ]);
    let odd = answering(json!({ "type": "rank", "order": [] })).await;
    let mut unreadable = jev(Some(&odd));
    unreadable.args([
        "noul",
        "Is it urgent?",
        "--state",
        "s",
        "--fail-under",
        "0.7",
    ]);

    let (network, auth, unreadable) = (run(network).await, run(auth).await, run(unreadable).await);

    assert_eq!((network.code, network.stdout.as_str()), (6, ""));
    assert_eq!((auth.code, auth.stdout.as_str()), (3, ""));
    assert_eq!(
        unreadable.code, 1,
        "an answer that cannot be compared is an error: {}",
        unreadable.stderr
    );
    for failed in [network, auth, unreadable] {
        assert!(
            json_of(&failed.stderr)["error"]["exit_code"]
                .as_u64()
                .unwrap()
                != 10
        );
        assert!(!failed.stderr.contains(SENTINEL_KEY));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_choice_gate_accepts_any_expected_option_and_checks_confidence() {
    let base = [
        "choice",
        "Which team?",
        "--state",
        "s",
        "--option",
        "billing=Payments",
        "--option",
        "sales=Pricing",
        "--option",
        "other",
    ];
    let cases: [(&str, f64, Vec<&str>, i32); 5] = [
        (
            "billing",
            0.9,
            vec!["--expect", "billing", "--expect", "sales"],
            0,
        ),
        (
            "sales",
            0.9,
            vec!["--expect", "billing", "--expect", "sales"],
            0,
        ),
        (
            "other",
            0.9,
            vec!["--expect", "billing", "--expect", "sales"],
            10,
        ),
        ("billing", 0.5, vec!["--min-confidence", "0.8"], 10),
        (
            "billing",
            0.5,
            vec!["--expect", "billing", "--min-confidence", "0.4"],
            0,
        ),
    ];
    for (winner, confidence, flags, exit) in cases {
        let server = answering(choice(winner, confidence)).await;
        let mut command = jev(Some(&server));
        command.args(base).args(&flags);

        let run = run(command).await;

        assert_eq!(
            run.code, exit,
            "{winner} {confidence} {flags:?}: {}",
            run.stderr
        );
        assert_eq!(json_of(&run.stdout)["choice"], winner);
    }
    let server = answering(choice("billing", 0.9)).await;
    assert_eq!(
        sent_after(&server, &base).await["questions"]["answer"]["criteria"],
        json!({ "billing": "Payments", "sales": "Pricing", "other": null }),
        "an option without a description is sent as null, in the order given"
    );
}

async fn sent_after(server: &MockServer, arguments: &[&str]) -> Value {
    let mut command = jev(Some(server));
    command.args(arguments);
    assert_eq!(run(command).await.code, 0);
    sent(server).await.remove(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_score_gate_checks_the_value_and_the_confidence() {
    let base = [
        "score",
        "How angry?",
        "--state",
        "s",
        "--level",
        "Calm",
        "--level",
        "Angry",
    ];
    let cases: [(f64, f64, Vec<&str>, i32); 4] = [
        (0.6, 0.9, vec!["--fail-over", "0.5"], 10),
        (0.4, 0.9, vec!["--fail-over", "0.5"], 0),
        (0.6, 0.9, vec!["--fail-under", "0.5"], 0),
        (
            0.6,
            0.3,
            vec!["--fail-under", "0.5", "--min-confidence", "0.5"],
            10,
        ),
    ];
    for (value, confidence, flags, exit) in cases {
        let server = answering(score(value, confidence)).await;
        let mut command = jev(Some(&server));
        command.args(base).args(&flags);

        assert_eq!(
            run(command).await.code,
            exit,
            "{value} {confidence} {flags:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_score_with_one_or_eleven_levels_exits_2_without_sending() {
    let server = answering(score(0.5, 0.9)).await;
    let eleven: Vec<String> = (0..11)
        .flat_map(|level| ["--level".to_owned(), format!("L{level}")])
        .collect();
    let mut one = jev(Some(&server));
    one.args(["score", "How angry?", "--state", "s", "--level", "Only"]);
    let mut many = jev(Some(&server));
    many.args(["score", "How angry?", "--state", "s"])
        .args(&eleven);

    let (one, many) = (run(one).await, run(many).await);

    for (run, rule) in [
        (one, "score-too-few-levels"),
        (many, "score-too-many-levels"),
    ] {
        assert_eq!((run.code, run.stdout.as_str()), (2, ""));
        assert_eq!(
            json_of(&run.stderr)["error"]["details"]["findings"][0]["rule"],
            rule
        );
    }
    assert!(
        sent(&server).await.is_empty(),
        "nothing may be sent for an invalid request"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn field_prints_the_bare_value_for_each_shortcut() {
    let cases: [(Value, Vec<&str>, &str, &str); 3] = [
        (noul(0.92), vec!["noul", "Urgent?"], "noul", "0.92\n"),
        (
            choice("billing", 0.9),
            vec![
                "choice", "Which?", "--option", "billing", "--option", "sales", "--option", "other",
            ],
            "choice",
            "billing\n",
        ),
        (
            score(0.6, 0.9),
            vec!["score", "How?", "--level", "Calm", "--level", "Angry"],
            "score",
            "0.6\n",
        ),
    ];
    for (answer, arguments, field, expected) in cases {
        let server = answering(answer).await;
        let mut command = jev(Some(&server));
        command
            .args(&arguments)
            .args(["--state", "s", "--field", field]);

        let run = run(command).await;

        assert_eq!(
            (run.code, run.stdout.as_str(), run.stderr.as_str()),
            (0, expected, ""),
            "{field}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_choice_without_a_way_out_warns_unless_told_not_to() {
    let server = answering(choice("billing", 0.9)).await;
    let arguments = [
        "choice", "Which?", "--state", "s", "--option", "billing", "--option", "sales",
    ];
    let mut warned = jev(Some(&server));
    warned.args(arguments);
    let mut silenced = jev(Some(&server));
    silenced.args(arguments).arg("--no-escape-warning");

    let (warned, silenced) = (run(warned).await, run(silenced).await);

    assert_eq!(warned.code, 0);
    let warning = &json_of(&warned.stderr)["warning"];
    assert!(
        warning["message"]
            .as_str()
            .unwrap()
            .contains("[choice-no-escape-option]"),
        "{warning}"
    );
    assert!(
        warning["hint"].as_str().unwrap().contains("other"),
        "{warning}"
    );
    assert_eq!((silenced.code, silenced.stderr.as_str()), (0, ""));
}

#[tokio::test(flavor = "multi_thread")]
async fn structured_instructions_and_criteria_come_from_files() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("shortcut-files-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let instructions = dir.join("instructions.yaml");
    let criteria = dir.join("criteria.json");
    std::fs::write(
        &instructions,
        "question: How risky is this change?\nfocus: [scope, blast radius]\n",
    )
    .unwrap();
    std::fs::write(
        &criteria,
        r#"["Safe", {"what": "Risky", "signals": ["touches billing"]}]"#,
    )
    .unwrap();
    let server = answering(score(0.4, 0.9)).await;
    let mut command = jev(Some(&server));
    command.args([
        "score",
        "--instructions-file",
        instructions.to_str().unwrap(),
        "--criteria-file",
        criteria.to_str().unwrap(),
        "--state",
        "diff",
    ]);

    assert_eq!(run(command).await.code, 0);

    let question = &sent(&server).await[0]["questions"]["answer"];
    assert_eq!(
        question["instructions"],
        json!({ "question": "How risky is this change?", "focus": ["scope", "blast radius"] })
    );
    assert_eq!(
        question["criteria"],
        json!(["Safe", { "what": "Risky", "signals": ["touches billing"] }])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn eval_asserts_must_all_hold_and_are_checked_before_sending() {
    let server = MockServer::start().await;
    let body = json!({ "model": "jev-1.13.0", "usage": { "input_tokens": 1, "output_tokens": 1 }, "answers": {
        "urgent": noul(0.92), "team": choice("billing", 0.5)
    }});
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let request = json!({ "state": "s", "questions": {
        "urgent": { "type": "noul", "instructions": "Urgent?" },
        "team": { "type": "choice", "instructions": "Which?", "criteria": { "billing": null, "sales": null, "other": null } }
    }});
    let cases: [(Vec<&str>, i32); 4] = [
        (
            vec!["--assert", "urgent >= 0.7", "--assert", "team == billing"],
            0,
        ),
        (
            vec![
                "--assert",
                "urgent >= 0.7",
                "--assert",
                "team.confidence >= 0.8",
            ],
            10,
        ),
        (vec!["--assert", "team != billing"], 10),
        (vec![], 0),
    ];
    for (flags, exit) in cases {
        let mut command = jev(Some(&server));
        command
            .args(["eval", "-f", "-"])
            .args(&flags)
            .write_stdin(request.to_string());

        let run = run(command).await;

        assert_eq!(run.code, exit, "{flags:?}: {}", run.stderr);
        let result = json_of(&run.stdout);
        assert_eq!(
            result["answers"]["urgent"]["noul"], 0.92,
            "the answers are printed either way"
        );
        assert_eq!(
            result.get("gate").is_some(),
            !flags.is_empty(),
            "`gate` appears only when a gate was asked for"
        );
    }
    let before = sent(&server).await.len();

    for (flag, message) in [
        ("urgnt >= 0.7", "there is no question `urgnt`"),
        ("team >= 0.5", "whose answer is an option"),
        ("team == biling", "`biling` is not one of the options"),
        ("nonsense", "there is no operator"),
    ] {
        let mut command = jev(Some(&server));
        command
            .args(["eval", "-f", "-", "--assert", flag])
            .write_stdin(request.to_string());

        let run = run(command).await;

        assert_eq!((run.code, run.stdout.as_str()), (2, ""), "{flag}");
        assert!(
            json_of(&run.stderr)["error"]["message"]
                .as_str()
                .unwrap()
                .contains(message),
            "{flag}: {}",
            run.stderr
        );
    }
    assert_eq!(
        sent(&server).await.len(),
        before,
        "a condition that cannot be decided costs nothing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_person_sees_the_answer_and_the_gate() {
    let server = answering(noul(0.31)).await;
    let mut command = jev(Some(&server));
    command.args([
        "noul",
        "Urgent?",
        "--state",
        "s",
        "--fail-under",
        "0.7",
        "-o",
        "table",
        "--ascii",
    ]);

    let run = run(command).await;

    assert_eq!(run.code, 10);
    assert!(
        run.stdout
            .starts_with("answer  noul  no  0.31  ######.............."),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("gate failed: answer >= 0.7 (does not hold, got 0.31)"),
        "{}",
        run.stdout
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn help_for_each_shortcut_shows_a_shell_if_and_the_exit_codes() {
    for shortcut in ["noul", "choice", "score"] {
        let mut command = jev(None);
        command.args([shortcut, "--help"]);

        let run = run(command).await;

        assert_eq!(run.code, 0);
        assert!(
            run.stdout.contains("if ") && run.stdout.contains(&format!("jev {shortcut} ")),
            "{shortcut}:\n{}",
            run.stdout
        );
        assert!(
            run.stdout.contains("\n  10   evaluated, and ")
                && run.stdout.contains("(never an error)"),
            "{shortcut}:\n{}",
            run.stdout
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn state_dash_is_a_usage_error_for_every_shortcut_before_anything_is_sent() {
    let server = answering(noul(0.9)).await;
    let shortcuts: [Vec<&str>; 3] = [
        vec!["noul", "Q?"],
        vec!["choice", "Q?", "--option", "x", "--option", "other"],
        vec!["score", "Q?", "--level", "Low", "--level", "High"],
    ];

    for shortcut in shortcuts {
        for (dry_run, stdin) in [(true, Some("{\"a\":1}")), (true, None), (false, None)] {
            let mut command = jev(Some(&server));
            command.args(&shortcut).args(["--state", "-"]);
            if dry_run {
                command.arg("--dry-run").env_remove("TYPESAFE_API_KEY");
            }
            if let Some(stdin) = stdin {
                command.write_stdin(stdin);
            }
            let run = run(command).await;

            let case = (&shortcut, dry_run, stdin);
            assert_eq!((run.code, run.stdout.as_str()), (2, ""), "{case:?}");
            assert!(
                json_of(&run.stderr)["error"]["hint"]
                    .as_str()
                    .unwrap()
                    .contains("--state-file -"),
                "{case:?}: {}",
                run.stderr
            );
        }
    }
    assert!(sent(&server).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_options_are_usage_errors() {
    let cases: [(Vec<&str>, &str); 3] = [
        (
            vec!["choice", "Which?", "--state", "s", "--option", "=no name"],
            "has no name",
        ),
        (
            vec![
                "choice", "Which?", "--state", "s", "--option", "a", "--option", "a=again",
            ],
            "given more than once",
        ),
        (
            vec![
                "choice", "Which?", "--state", "s", "--option", "a", "--option", "other",
                "--expect", "b",
            ],
            "`b` is not one of the options",
        ),
    ];
    for (arguments, message) in cases {
        let mut command = jev(None);
        command.args(&arguments);

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
}
