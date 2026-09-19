//! The help and `jev spec` as a contract: every `--help` and the spec are snapshots, so a change to
//! either shows up in review, and neither may touch the network or the configuration.
//!
//! After an intended change, refresh the snapshots and review the diff:
//! `JEV_UPDATE_SNAPSHOTS=1 cargo test -p jev-cli --all-features --test help`.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use assert_cmd::Command;
use serde_json::Value;
use wiremock::MockServer;

/// Set to refresh the snapshots instead of comparing with them.
const UPDATE: &str = "JEV_UPDATE_SNAPSHOTS";

/// `jev`, isolated from the environment of whoever runs the tests, with no configuration, no key
/// and an API address nothing listens on.
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
    command.env("TYPESAFE_BASE_URL", "http://127.0.0.1:9");
    // Help wraps at the terminal's width, or at `COLUMNS`; the snapshots are at the widest.
    command.env("COLUMNS", "100");
    command
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(command: &mut Command) -> Run {
    let output = command.timeout(Duration::from_secs(60)).output().unwrap();
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("not JSON ({error}): {text:?}"))
}

fn snapshots() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

/// Text as the repository keeps it: no trailing spaces (clap leaves some on blank lines inside a
/// flag's help), and one newline at the end. The git hooks would strip them otherwise.
fn normalise(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    format!("{}\n", lines.join("\n").trim_end())
}

/// Compares `actual` with the snapshot at `path`, or writes it there when [`UPDATE`] is set.
/// Returns a description of the difference, if there is one.
fn check_snapshot(path: &Path, actual: &str) -> Option<String> {
    let actual = normalise(actual);
    let actual = actual.as_str();
    if std::env::var_os(UPDATE).is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, actual).unwrap();
        return None;
    }
    let Ok(expected) = std::fs::read_to_string(path) else {
        return Some(format!("{} does not exist", path.display()));
    };
    // A Windows checkout may have turned the newlines into CRLF.
    let expected = normalise(&expected);
    if expected == actual {
        return None;
    }
    let line = expected
        .lines()
        .zip(actual.lines())
        .position(|(expected, actual)| expected != actual)
        .unwrap_or_else(|| expected.lines().count().min(actual.lines().count()));
    Some(format!(
        "{} differs from line {}:\n  expected: {:?}\n  actual:   {:?}",
        path.display(),
        line + 1,
        expected.lines().nth(line).unwrap_or("<end>"),
        actual.lines().nth(line).unwrap_or("<end>"),
    ))
}

fn fail_on(differences: &[String]) {
    assert!(
        differences.is_empty(),
        "{}\n\nIf the change is intended, refresh the snapshots and review the diff:\n  \
{UPDATE}=1 cargo test -p jev-cli --all-features --test help\n",
        differences.join("\n\n")
    );
}

/// Every visible command, groups included, as the words after `jev`.
fn command_paths() -> Vec<Vec<String>> {
    fn walk(command: &clap::Command, path: &[String], paths: &mut Vec<Vec<String>>) {
        paths.push(path.to_vec());
        for sub in command.get_subcommands().filter(|sub| !sub.is_hide_set()) {
            let mut below = path.to_vec();
            below.push(sub.get_name().to_owned());
            walk(sub, &below, paths);
        }
    }
    let mut paths = Vec::new();
    walk(&jev_cli::command(), &[], &mut paths);
    paths
}

fn snapshot_name(path: &[String]) -> String {
    std::iter::once("jev")
        .chain(path.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join("-")
}

#[test]
fn the_help_of_every_command_matches_its_snapshot() {
    let dir = snapshots().join("help");
    let mut differences = Vec::new();
    let mut expected_files = Vec::new();

    for path in command_paths() {
        let name = format!("{}.txt", snapshot_name(&path));
        let help = run(jev().args(&path).arg("--help"));
        assert_eq!(
            (help.code, help.stderr.as_str()),
            (0, ""),
            "jev {} --help",
            path.join(" ")
        );
        differences.extend(check_snapshot(&dir.join(&name), &help.stdout));
        expected_files.push(name);
    }

    if std::env::var_os(UPDATE).is_none() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            if !expected_files.contains(&name) {
                differences.push(format!(
                    "{} is the snapshot of a command that no longer exists: delete it",
                    dir.join(name).display()
                ));
            }
        }
    }
    fail_on(&differences);
}

#[test]
fn the_spec_matches_its_snapshot() {
    let spec = run(jev().arg("spec"));
    assert_eq!((spec.code, spec.stderr.as_str()), (0, ""));

    // The version changes with every release; everything else is the contract.
    let mut document = json(&spec.stdout);
    assert_eq!(document["version"], env!("CARGO_PKG_VERSION"));
    document["version"] = Value::from("<version>");
    let text = format!("{}\n", serde_json::to_string_pretty(&document).unwrap());

    fail_on(
        &check_snapshot(&snapshots().join("spec.json"), &text)
            .into_iter()
            .collect::<Vec<_>>(),
    );
}

#[test]
fn the_spec_describes_every_command_with_typed_flags_and_examples() {
    let spec = json(&run(jev().arg("spec")).stdout);

    assert_eq!(spec["spec_version"], 1);
    assert_eq!(spec["name"], "jev");
    let paths: Vec<&str> = spec["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|command| command["path"].as_str().unwrap())
        .collect();
    // PRD section 6.
    for expected in [
        "eval",
        "noul",
        "choice",
        "score",
        "validate",
        "batch run",
        "models list",
        "auth login",
        "auth status",
        "auth logout",
        "config get",
        "config set",
        "config unset",
        "config list",
        "config path",
        "profile list",
        "profile use",
        "profile create",
        "profile delete",
        "schema request",
        "schema questions",
        "schema batch-record",
        "schema output",
        "schema error",
        "spec",
        "mcp serve",
        "update",
        "completion",
        "version",
    ] {
        assert!(
            paths.contains(&expected),
            "{expected} is missing: {paths:?}"
        );
    }

    for command in spec["commands"].as_array().unwrap() {
        let path = command["path"].as_str().unwrap();
        if command["implemented"] == false {
            continue;
        }
        let examples = command["examples"].as_array().unwrap();
        assert!(
            (2..=4).contains(&examples.len()),
            "{path}: {} examples",
            examples.len()
        );
        assert!(
            !command["exit_codes"].as_array().unwrap().is_empty(),
            "{path}"
        );
        for field in ["when_to_use", "input", "output", "usage", "about"] {
            assert!(
                command[field].as_str().is_some_and(|text| !text.is_empty()),
                "{path}: {field}"
            );
        }
        for flag in command["flags"].as_array().unwrap() {
            assert!(flag["type"].is_string(), "{path}: {flag}");
            assert!(flag.get("default").is_some(), "{path}: {flag}");
        }
    }
    let codes: Vec<u64> = spec["exit_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|code| code["code"].as_u64().unwrap())
        .collect();
    assert_eq!(codes, [0, 1, 2, 3, 4, 5, 6, 7, 10, 11, 20, 130]);
}

#[test]
fn the_spec_is_data_in_every_format() {
    let yaml = run(jev().args(["spec", "-o", "yaml"]));
    let field = run(jev().args(["spec", "--field", "spec_version"]));

    assert_eq!((yaml.code, yaml.stderr.as_str()), (0, ""));
    assert!(
        yaml.stdout.starts_with("spec_version: 1\n"),
        "{}",
        yaml.stdout
    );
    assert_eq!(
        (field.code, field.stdout.as_str(), field.stderr.as_str()),
        (0, "1\n", "")
    );
}

#[test]
fn the_help_of_the_shortcuts_says_when_to_pick_each_over_the_others() {
    for (shortcut, others) in [
        ("noul", ["jev choice", "jev score"]),
        ("choice", ["jev noul", "jev score"]),
        ("score", ["jev noul", "jev choice"]),
    ] {
        let help = run(jev().args([shortcut, "--help"]));

        let when = help
            .stdout
            .split("When to use:")
            .nth(1)
            .and_then(|rest| rest.split("Input:").next())
            .unwrap_or_else(|| panic!("{shortcut} has no when-to-use section:\n{}", help.stdout));
        for other in others {
            assert!(
                when.contains(other),
                "{shortcut} does not mention {other}:\n{when}"
            );
        }
    }
}

#[test]
fn short_help_keeps_the_exit_codes_and_examples() {
    let help = run(jev().args(["noul", "-h"]));

    assert_eq!(help.code, 0);
    assert!(help.stdout.contains("Exit codes:"), "{}", help.stdout);
    assert!(help.stdout.contains("Examples:"), "{}", help.stdout);
}

#[tokio::test(flavor = "multi_thread")]
async fn spec_and_help_touch_neither_the_network_nor_the_configuration() {
    // Any request at all would be recorded, and the configuration directory must stay absent.
    let server = MockServer::start().await;
    let config = Path::new(env!("CARGO_TARGET_TMPDIR")).join("help-must-not-create-this");
    let _ = std::fs::remove_dir_all(&config);

    for arguments in [
        vec!["spec"],
        vec!["--help"],
        vec!["noul", "--help"],
        vec!["auth", "login", "--help"],
    ] {
        let mut command = jev();
        command
            .args(&arguments)
            .env("TYPESAFE_BASE_URL", server.uri())
            .env("TYPESAFE_API_KEY", "sentinel-key-do-not-leak-7a1e")
            .env("JEV_CONFIG_DIR", &config);
        let run = tokio::task::spawn_blocking(move || run(&mut command))
            .await
            .unwrap();

        assert_eq!((run.code, run.stderr.as_str()), (0, ""), "{arguments:?}");
        assert!(!run.stdout.contains("sentinel-key"), "{arguments:?}");
    }

    assert_eq!(
        server.received_requests().await.unwrap_or_default().len(),
        0
    );
    assert!(!config.exists(), "{} was created", config.display());
}

#[test]
fn spec_works_when_the_configuration_is_broken() {
    let config = Path::new(env!("CARGO_TARGET_TMPDIR")).join("help-broken-config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("config.toml"), "this is = = not toml").unwrap();

    let spec = run(jev().arg("spec").env("JEV_CONFIG_DIR", &config));

    assert_eq!(spec.code, 0, "{}", spec.stderr);
    assert_eq!(json(&spec.stdout)["name"], "jev");
}

#[test]
fn a_mistyped_command_or_flag_is_answered_with_the_nearest_one() {
    let command = run(jev().args(["nuol", "Is it?", "-o", "json"]));
    let flag = run(jev().args(["noul", "Is it?", "--fail-undr", "0.5", "-o", "json"]));

    for (run, nearest) in [(command, "noul"), (flag, "--fail-under")] {
        assert_eq!(run.code, 2);
        assert_eq!(run.stdout, "");
        let error = &json(&run.stderr)["error"];
        assert!(
            error["hint"].as_str().unwrap().contains(nearest),
            "{nearest}: {error}"
        );
    }
}
