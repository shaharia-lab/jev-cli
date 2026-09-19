//! `jev` from the outside: what reaches stdout, what reaches stderr, and the exit code.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use assert_cmd::Command;
use serde_json::Value;

/// `jev`, isolated from the environment of whoever runs the tests (CI sets `CI=true`, a developer
/// may have `NO_COLOR` or a real API key set).
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
        "TYPESAFE_LOG_LEVEL",
    ] {
        command.env_remove(variable);
    }
    // Never the real user configuration: an empty directory nothing writes to.
    command.env(
        "JEV_CONFIG_DIR",
        std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("no-config"),
    );
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

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("not JSON ({error}): {text:?}"))
}

#[test]
fn version_prints_json_on_a_pipe_and_nothing_on_stderr() {
    let run = run(jev().arg("version"));

    assert_eq!(run.code, 0);
    assert_eq!(run.stderr, "");
    let info = json(&run.stdout);
    assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));
    for field in ["commit", "build_date", "target", "client_version"] {
        assert!(
            info[field].as_str().is_some_and(|value| !value.is_empty()),
            "{field}: {info}"
        );
    }
}

#[test]
fn every_format_is_available_by_flag_and_by_environment_variable() {
    let table = run(jev().args(["version", "-o", "table"]));
    let yaml = run(jev().args(["version", "--output", "yaml"]));
    let jsonl = run(jev().args(["-o", "jsonl", "version"]));
    let from_env = run(jev().arg("version").env("JEV_OUTPUT", "yaml"));

    assert!(
        table
            .stdout
            .starts_with(concat!("jev ", env!("CARGO_PKG_VERSION"), "\n")),
        "{}",
        table.stdout
    );
    assert!(yaml.stdout.starts_with("version: "), "{}", yaml.stdout);
    assert_eq!(jsonl.stdout.lines().count(), 1);
    assert_eq!(json(&jsonl.stdout)["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(from_env.stdout, yaml.stdout);
    for run in [table, yaml, jsonl, from_env] {
        assert_eq!((run.code, run.stderr.as_str()), (0, ""));
    }
}

#[test]
fn field_prints_one_bare_value_and_a_missing_path_exits_2() {
    let found = run(jev().args(["version", "--field", "version"]));
    assert_eq!(
        (found.code, found.stdout.as_str(), found.stderr.as_str()),
        (0, concat!(env!("CARGO_PKG_VERSION"), "\n"), "")
    );

    let missing = run(jev().args(["version", "--field", "nope"]));
    assert_eq!(missing.code, 2);
    assert_eq!(missing.stdout, "", "stdout stays clean on an error");
    let error = &json(&missing.stderr)["error"];
    assert_eq!(error["code"], "usage");
    assert_eq!(error["exit_code"], 2);
    assert!(
        error["hint"]
            .as_str()
            .unwrap()
            .starts_with("available there: version, commit"),
        "{error}"
    );
}

#[test]
fn no_command_is_a_usage_error_with_help_for_a_person_and_a_json_error_for_a_program() {
    let person = run(jev().args(["-o", "table"]));
    let program = run(&mut jev());

    assert_eq!((person.code, person.stdout.as_str()), (2, ""));
    assert!(
        person.stderr.contains("Usage: jev") && person.stderr.contains("eval"),
        "{}",
        person.stderr
    );

    assert_eq!((program.code, program.stdout.as_str()), (2, ""));
    let error = &json(&program.stderr)["error"];
    assert_eq!(error["message"], "no command was given");
    assert!(
        error["hint"].as_str().unwrap().contains("jev --help"),
        "{error}"
    );
}

#[test]
fn asked_for_help_and_version_go_to_stdout_and_succeed() {
    let help = run(jev().arg("--help"));
    let sub_help = run(jev().args(["batch", "run", "--help"]));
    let version = run(jev().arg("--version"));

    assert_eq!((help.code, help.stderr.as_str()), (0, ""));
    for expected in [
        "noul",
        "choice",
        "score",
        "Global options",
        "--output",
        "--no-input",
        "unofficial",
    ] {
        assert!(
            help.stdout.contains(expected),
            "help lacks {expected:?}:\n{}",
            help.stdout
        );
    }
    assert!(
        !help
            .stdout
            .lines()
            .any(|line| line.trim_start().starts_with("debug ")),
        "the `debug` test-hook command is hidden:\n{}",
        help.stdout
    );
    assert_eq!(sub_help.code, 0);
    assert!(
        sub_help.stdout.contains("Usage: jev batch run"),
        "{}",
        sub_help.stdout
    );
    assert_eq!(
        (version.code, version.stdout.as_str()),
        (0, concat!("jev ", env!("CARGO_PKG_VERSION"), "\n"))
    );
}

#[test]
fn help_names_the_program_jev_whatever_the_file_is_called() {
    // On Windows the file is `jev.exe`, and clap would otherwise name the program after it.
    let renamed = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("renamed-jev.exe");
    // A hard link, not a copy. Copying opens the new file for writing, and if another test forks
    // at that moment its child inherits the handle; running the copy then fails on Linux with
    // "Text file busy". A link never opens the file for writing, so there is nothing to race.
    let _ = std::fs::remove_file(&renamed);
    std::fs::hard_link(assert_cmd::cargo::cargo_bin("jev"), &renamed).unwrap();

    let output = std::process::Command::new(&renamed)
        .args(["batch", "run", "--help"])
        .output()
        .unwrap();

    let help = String::from_utf8(output.stdout).unwrap();
    assert!(output.status.success());
    assert!(help.contains("Usage: jev batch run"), "{help}");
    assert!(!help.contains("renamed-jev"), "{help}");
}

#[test]
fn an_unknown_command_or_flag_suggests_the_nearest_one() {
    let command = run(jev().args(["evl", "-o", "table"]));
    let flag = run(jev().args(["version", "--outptu", "json", "-o", "table"]));

    assert_eq!((command.code, command.stdout.as_str()), (2, ""));
    assert!(command.stderr.contains("'eval'"), "{}", command.stderr);
    assert_eq!((flag.code, flag.stdout.as_str()), (2, ""));
    assert!(flag.stderr.contains("'--output'"), "{}", flag.stderr);
}

#[test]
fn a_usage_error_is_a_json_object_on_stderr_when_the_output_is_for_a_program() {
    let piped = run(jev().arg("evl"));
    let by_flag = run(jev().args(["version", "--timeout", "soon", "--output=json"]));

    for run in [&piped, &by_flag] {
        assert_eq!((run.code, run.stdout.as_str()), (2, ""));
        assert_eq!(
            run.stderr.lines().count(),
            1,
            "one JSON object: {}",
            run.stderr
        );
    }
    let error = &json(&piped.stderr)["error"];
    assert_eq!(error["code"], "usage");
    assert!(
        error["message"].as_str().unwrap().contains("evl"),
        "{error}"
    );
    assert!(
        error["hint"].as_str().unwrap().contains("eval"),
        "the suggestion is in the hint: {error}"
    );
    assert!(
        json(&by_flag.stderr)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("soon")
    );
}

#[test]
fn commands_without_behaviour_say_so_whatever_flags_follow() {
    let cases = [
        (vec!["update", "--check"], "update", 25),
        (vec!["schema", "batch-record"], "schema batch-record", 16),
        (vec!["completion", "zsh"], "completion", 17),
    ];

    for (arguments, name, issue) in cases {
        let run = run(jev().args(&arguments));

        assert_eq!((run.code, run.stdout.as_str()), (2, ""), "{arguments:?}");
        let error = &json(&run.stderr)["error"];
        assert_eq!(error["code"], "not_implemented");
        assert_eq!(
            error["message"],
            format!("`jev {name}` is not implemented yet")
        );
        assert!(
            error["hint"]
                .as_str()
                .unwrap()
                .ends_with(&format!("/issues/{issue}")),
            "{error}"
        );
    }
}

#[test]
fn verbose_logging_never_touches_stdout() {
    let run = run(jev().args(["version", "-vv"]));

    assert_eq!(run.code, 0);
    assert_eq!(json(&run.stdout)["version"], env!("CARGO_PKG_VERSION"));
}

#[cfg(not(feature = "internal-test-hooks"))]
#[test]
fn release_builds_do_not_contain_the_test_hooks() {
    let run = run(jev().args(["debug", "panic"]));

    assert_eq!(run.code, 2);
    assert!(
        json(&run.stderr)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("debug")
    );
}

#[cfg(feature = "internal-test-hooks")]
mod with_hooks {
    use serde_json::Value;

    use super::{jev, json, run};

    const RESPONSE: &str = include_str!("fixtures/response.json");

    #[test]
    fn a_result_is_the_documented_envelope_with_answers_passed_through() {
        let run = run(jev()
            .args([
                "debug",
                "render",
                "--request-id",
                "req_1",
                "--latency-ms",
                "905",
            ])
            .write_stdin(RESPONSE));

        assert_eq!((run.code, run.stderr.as_str()), (0, ""));
        let envelope = json(&run.stdout);
        let keys: Vec<&str> = envelope
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
        assert_eq!(envelope["answers"], json(RESPONSE)["answers"]);
        assert_eq!(envelope["requested_model"], "jev-latest");
        assert_eq!(envelope["request_id"], "req_1");
    }

    #[test]
    fn field_prints_the_bare_probability() {
        let noul = run(jev()
            .args(["debug", "render", "--field", "answers.is_urgent.noul"])
            .write_stdin(RESPONSE));
        let choice = run(jev()
            .args([
                "debug",
                "render",
                "--field",
                "answers.department.choice",
                "-o",
                "yaml",
            ])
            .write_stdin(RESPONSE));
        let missing = run(jev()
            .args(["debug", "render", "--field", "answers.x.noul"])
            .write_stdin(RESPONSE));

        assert_eq!(
            (noul.code, noul.stdout.as_str(), noul.stderr.as_str()),
            (0, "0.92\n", "")
        );
        assert_eq!((choice.code, choice.stdout.as_str()), (0, "technical\n"));
        assert_eq!((missing.code, missing.stdout.as_str()), (2, ""));
        let error = &json(&missing.stderr)["error"];
        assert_eq!(
            error["message"],
            "--field `answers.x.noul`: there is no `x` at `answers`"
        );
        assert_eq!(error["hint"], "available there: is_urgent, department");
    }

    #[test]
    fn the_human_form_is_legible_in_plain_ascii_and_has_no_colour_on_a_pipe() {
        let run = run(jev()
            .args(["debug", "render", "-o", "table", "--ascii"])
            .write_stdin(RESPONSE));

        assert_eq!(run.code, 0);
        assert!(run.stdout.is_ascii(), "{}", run.stdout);
        assert!(
            run.stdout.contains("technical  0.85  #################..."),
            "{}",
            run.stdout
        );
        assert!(!run.stdout.contains('\u{1b}'), "no colour on a pipe");
    }

    #[cfg(unix)]
    #[test]
    fn unicode_is_used_only_when_the_locale_can_show_it() {
        let utf8 = run(jev()
            .args(["debug", "render", "-o", "table"])
            .env("LANG", "en_GB.UTF-8")
            .write_stdin(RESPONSE));
        let c_locale = run(jev()
            .args(["debug", "render", "-o", "table"])
            .env("LANG", "C")
            .write_stdin(RESPONSE));
        let unset = run(jev()
            .args(["debug", "render", "-o", "table"])
            .write_stdin(RESPONSE));

        assert!(utf8.stdout.contains('█'), "{}", utf8.stdout);
        assert!(c_locale.stdout.is_ascii(), "{}", c_locale.stdout);
        assert!(unset.stdout.is_ascii(), "{}", unset.stdout);
    }

    #[test]
    fn api_errors_exit_with_their_documented_code_and_the_json_error_shape() {
        let cases = [
            ("authentication", 3, "authentication", Some(401), false),
            (
                "permission-denied",
                3,
                "permission_denied",
                Some(403),
                false,
            ),
            ("unprocessable", 4, "api_rejected", Some(422), false),
            ("not-found", 4, "not_found", Some(404), false),
            ("rate-limit", 5, "rate_limited", Some(429), true),
            ("overloaded", 5, "overloaded", Some(529), true),
            ("server", 6, "server_error", Some(500), true),
            ("timeout", 6, "timeout", None, true),
            ("connection", 6, "connection", None, true),
            ("invalid-response", 1, "invalid_response", Some(200), false),
        ];

        for (kind, exit, code, status, retryable) in cases {
            let run = run(jev().args(["debug", "error", "--kind", kind]));

            assert_eq!((run.code, run.stdout.as_str()), (exit, ""), "{kind}");
            let error = json(&run.stderr)["error"].clone();
            let fields: Vec<&str> = error
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                fields,
                [
                    "code",
                    "exit_code",
                    "error_type",
                    "message",
                    "hint",
                    "request_id",
                    "http_status",
                    "retryable",
                    "details",
                ],
                "{kind}"
            );
            assert_eq!(error["code"], code, "{kind}");
            assert_eq!(error["exit_code"], exit, "{kind}");
            assert_eq!(
                error["http_status"],
                status.map_or(Value::Null, Value::from),
                "{kind}"
            );
            assert_eq!(error["retryable"], retryable, "{kind}");
            assert_eq!(error["request_id"], "req_test", "{kind}");
            assert!(
                error["hint"].as_str().is_some_and(|hint| !hint.is_empty()),
                "{kind}"
            );
        }
    }

    #[test]
    fn errors_for_a_person_are_text_on_stderr() {
        let run = run(jev().args(["debug", "error", "--kind", "authentication", "-o", "table"]));

        assert_eq!((run.code, run.stdout.as_str()), (3, ""));
        assert_eq!(
            run.stderr,
            "error: Invalid API key.\n  hint: check the API key: set TYPESAFE_API_KEY, or run `jev auth login`\n  request id: req_test\n"
        );
    }

    #[test]
    fn a_would_be_prompt_fails_fast_when_nobody_can_answer() {
        let piped = run(jev().args(["debug", "prompt"]));
        let error = &json(&piped.stderr)["error"];

        assert_eq!((piped.code, piped.stdout.as_str()), (2, ""));
        assert_eq!(error["code"], "input_required");
        assert_eq!(
            error["message"],
            "cannot ask for the API key: stdin is not a terminal"
        );
        assert!(
            error["hint"].as_str().unwrap().contains("TYPESAFE_API_KEY"),
            "{error}"
        );

        let flag = run(jev().args(["debug", "prompt", "--no-input"]));
        let variable = run(jev().args(["debug", "prompt"]).env("JEV_NO_INPUT", "1"));
        let ci = run(jev().args(["debug", "prompt"]).env("CI", "true"));
        for (run, reason) in [
            (flag, "--no-input"),
            (variable, "--no-input"),
            (ci, "CI is set"),
        ] {
            assert_eq!(run.code, 2);
            assert!(
                json(&run.stderr)["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains(reason),
                "{}",
                run.stderr
            );
        }
    }

    #[test]
    fn a_panic_exits_1_with_a_bug_report_hint_in_the_format_asked_for() {
        let machine = run(jev().args(["debug", "panic"]));
        let human = run(jev().args(["debug", "panic", "-o", "table"]));

        assert_eq!((machine.code, machine.stdout.as_str()), (1, ""));
        let error = &json(&machine.stderr)["error"];
        assert_eq!(error["code"], "internal");
        assert_eq!(error["exit_code"], 1);
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .starts_with("jev crashed at "),
            "{error}"
        );
        assert!(
            error["hint"]
                .as_str()
                .unwrap()
                .contains("github.com/shaharia-lab/jev-cli/issues"),
            "{error}"
        );

        assert_eq!((human.code, human.stdout.as_str()), (1, ""));
        assert!(
            human.stderr.starts_with("error: jev crashed at "),
            "{}",
            human.stderr
        );
        assert!(
            !human.stderr.contains("RUST_BACKTRACE"),
            "Rust's own panic message is replaced: {}",
            human.stderr
        );
    }
}
