//! `jev config` and `jev profile` from the outside, always against a scratch `JEV_CONFIG_DIR`.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use assert_cmd::Command;
use serde_json::{Value, json};

/// A scratch configuration directory, unique to one test.
struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "config-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(unique);
        let _ = fs::remove_dir_all(&dir);
        Self(dir)
    }

    fn with_config(name: &str, toml: &str) -> Self {
        let sandbox = Self::new(name);
        fs::create_dir_all(&sandbox.0).unwrap();
        fs::write(sandbox.file(), toml).unwrap();
        sandbox
    }

    fn file(&self) -> PathBuf {
        self.0.join("config.toml")
    }

    fn text(&self) -> String {
        fs::read_to_string(self.file()).unwrap()
    }

    /// `jev`, isolated from the environment of whoever runs the tests.
    fn jev(&self) -> Command {
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
        command.env("JEV_CONFIG_DIR", &self.0);
        command
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
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

fn ok(command: &mut Command) -> String {
    let run = run(command);
    assert_eq!(
        (run.code, run.stderr.as_str()),
        (0, ""),
        "stdout: {}",
        run.stdout
    );
    run.stdout
}

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("not JSON ({error}): {text:?}"))
}

/// The `settings` of `jev config list -o json` as `key -> (value, source, origin)`.
fn listed(output: &str) -> Vec<(String, Value, String, Value)> {
    json_of(output)
        .get("settings")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["key"].as_str().unwrap().to_owned(),
                row["value"].clone(),
                row["source"].as_str().unwrap().to_owned(),
                row["origin"].clone(),
            )
        })
        .collect()
}

#[test]
fn with_no_configuration_every_setting_is_its_default() {
    let sandbox = Sandbox::new("defaults");

    let output = ok(sandbox.jev().args(["config", "list"]));

    let expected = [
        ("base_url", json!("https://api.typesafe.ai")),
        ("model", json!("jev-latest")),
        ("output", Value::Null),
        ("timeout", json!("30s")),
        ("max_retries", json!(2)),
        ("concurrency", json!(4)),
        ("warn_unpinned", json!(true)),
    ];
    let rows = listed(&output);
    assert_eq!(rows.len(), expected.len());
    for ((key, value, source, origin), (expected_key, expected_value)) in rows.iter().zip(&expected)
    {
        assert_eq!(
            (key.as_str(), value, source.as_str(), origin),
            (*expected_key, expected_value, "default", &Value::Null)
        );
    }
    assert_eq!(json_of(&output)["profile"]["value"], "default");
    assert!(!sandbox.file().exists(), "reading never creates the file");
}

#[test]
fn config_list_reports_the_value_and_the_source_of_every_key() {
    let sandbox = Sandbox::with_config(
        "sources",
        "[profiles.default]\nmodel = \"profile-model\"\nbase_url = \"https://profile.example\"\ntimeout = 45\nconcurrency = 8\n",
    );

    let output = ok(sandbox
        .jev()
        .args([
            "config",
            "list",
            "--output",
            "json",
            "--model",
            "flag-model",
        ])
        .env("TYPESAFE_BASE_URL", "https://env.example")
        .env("TYPESAFE_DEFAULT_MODEL", "env-model"));

    assert_eq!(
        listed(&output),
        [
            (
                "base_url".to_owned(),
                json!("https://env.example"),
                "env".to_owned(),
                json!("TYPESAFE_BASE_URL")
            ),
            (
                "model".to_owned(),
                json!("flag-model"),
                "flag".to_owned(),
                json!("--model")
            ),
            (
                "output".to_owned(),
                json!("json"),
                "flag".to_owned(),
                json!("--output")
            ),
            (
                "timeout".to_owned(),
                json!("45s"),
                "profile".to_owned(),
                json!("default")
            ),
            (
                "max_retries".to_owned(),
                json!(2),
                "default".to_owned(),
                Value::Null
            ),
            (
                "concurrency".to_owned(),
                json!(8),
                "profile".to_owned(),
                json!("default")
            ),
            (
                "warn_unpinned".to_owned(),
                json!(true),
                "default".to_owned(),
                Value::Null
            ),
        ]
    );
}

#[test]
fn the_human_listing_shows_where_each_value_comes_from() {
    let sandbox = Sandbox::with_config("human", "[profiles.default]\nmodel = \"jev-1.13.0\"\n");

    let output = ok(sandbox
        .jev()
        .args(["config", "list", "-o", "table"])
        .env("TYPESAFE_BASE_URL", "http://127.0.0.1:4010"));

    for line in [
        "SETTING        VALUE",
        "profile        default                default",
        "base_url       http://127.0.0.1:4010  env TYPESAFE_BASE_URL",
        "model          jev-1.13.0             profile `default`",
        "output         table                  flag --output",
        "timeout        30s                    default",
    ] {
        assert!(output.contains(line), "missing {line:?} in:\n{output}");
    }
    assert!(output.contains("config file: "), "{output}");
}

#[test]
fn set_get_and_unset_round_trip_and_keep_a_persons_comments() {
    let sandbox = Sandbox::with_config(
        "round-trip",
        "# mine\n[profiles.default]\nmodel = \"old\"  # pinned\n",
    );

    let set = json_of(&ok(sandbox.jev().args([
        "config",
        "set",
        "model",
        "jev-1.13.0",
    ])));
    assert_eq!(
        set,
        json!({ "profile": "default", "key": "model", "value": "jev-1.13.0", "changed": true })
    );
    ok(sandbox.jev().args(["config", "set", "Max-Retries", "5"]));
    ok(sandbox.jev().args(["config", "set", "timeout", "1.5"]));

    assert_eq!(
        ok(sandbox
            .jev()
            .args(["config", "get", "model", "-o", "table"])),
        "jev-1.13.0\n"
    );
    assert_eq!(
        ok(sandbox
            .jev()
            .args(["config", "get", "timeout", "--field", "value"])),
        "1500ms\n"
    );
    let got = json_of(&ok(sandbox.jev().args(["config", "get", "max_retries"])));
    assert_eq!(
        got,
        json!({
            "key": "max_retries",
            "value": 5,
            "source": "profile",
            "origin": "default",
            "description": "retries after the first attempt"
        })
    );

    let text = sandbox.text();
    assert!(text.starts_with("# mine\n"), "{text}");
    assert!(
        text.contains("model = \"jev-1.13.0\"  # pinned\n"),
        "{text}"
    );
    assert!(
        text.contains("max_retries = 5\n") && text.contains("timeout = \"1500ms\"\n"),
        "{text}"
    );
    assert!(
        !text.contains("schema_version"),
        "a file a person wrote is not given keys they did not ask for: {text}"
    );

    let unset = json_of(&ok(sandbox.jev().args(["config", "unset", "model"])));
    assert_eq!(unset["changed"], true);
    assert_eq!(
        json_of(&ok(sandbox.jev().args(["config", "unset", "model"])))["changed"],
        false
    );
    assert_eq!(
        ok(sandbox
            .jev()
            .args(["config", "get", "model", "-o", "table"])),
        "jev-latest\n"
    );
}

#[test]
fn bad_names_and_values_are_usage_errors_that_explain_themselves() {
    let sandbox = Sandbox::new("bad-input");
    let cases: [(&[&str], &str, &str); 5] = [
        (
            &["config", "set", "modle", "x"],
            "`modle` is not a setting",
            "did you mean `model`?",
        ),
        (
            &["config", "set", "timeout", "soon"],
            "`soon` is not a valid `timeout`",
            "time allowed per attempt",
        ),
        (
            &["config", "set", "base_url", "http://example.com"],
            "must use https://",
            "API root",
        ),
        (
            &["config", "set", "concurrency", "0"],
            "whole number from 1 to 64",
            "parallel requests",
        ),
        (
            &["config", "get", "nonsense"],
            "`nonsense` is not a setting",
            "the settings are: base_url, model",
        ),
    ];

    for (arguments, message, hint) in cases {
        let run = run(sandbox.jev().args(arguments));

        assert_eq!((run.code, run.stdout.as_str()), (2, ""), "{arguments:?}");
        let error = &json_of(&run.stderr)["error"];
        assert!(
            error["message"].as_str().unwrap().contains(message),
            "{arguments:?}: {error}"
        );
        assert!(
            error["hint"].as_str().unwrap().contains(hint),
            "{arguments:?}: {error}"
        );
    }
    assert!(!sandbox.file().exists(), "a rejected change writes nothing");
}

#[test]
fn a_credential_is_refused_without_echoing_it_and_never_reaches_the_file() {
    let sandbox = Sandbox::new("secrets");
    let secret = "sentinel-key-do-not-store-9d2c";

    for key in ["api_key", "TYPESAFE_API_KEY", "token"] {
        let run = run(sandbox.jev().args(["config", "set", key, secret]));

        assert_eq!(run.code, 2);
        assert!(
            !run.stderr.contains(secret) && !run.stdout.contains(secret),
            "{}",
            run.stderr
        );
        let error = &json_of(&run.stderr)["error"];
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("credentials are never stored"),
            "{error}"
        );
        assert!(
            error["hint"].as_str().unwrap().contains("jev auth login"),
            "{error}"
        );
    }
    assert!(!sandbox.file().exists());
}

#[test]
fn an_unknown_key_is_a_warning_on_stderr_and_the_command_still_succeeds() {
    let sandbox = Sandbox::with_config(
        "unknown-key",
        "colour = true\n[profiles.default]\nmodle = \"x\"\nmodel = \"jev-1.13.0\"\n",
    );

    let person = run(sandbox
        .jev()
        .args(["config", "get", "model", "-o", "table"]));
    let program = run(sandbox.jev().args(["config", "get", "model"]));
    let quiet = run(sandbox.jev().args(["config", "get", "model", "--quiet"]));

    assert_eq!((person.code, person.stdout.as_str()), (0, "jev-1.13.0\n"));
    assert!(
        person.stderr.contains("warning: ")
            && person.stderr.contains("unknown key `colour` is ignored"),
        "{}",
        person.stderr
    );
    assert!(
        person.stderr.contains("hint: did you mean `model`?"),
        "{}",
        person.stderr
    );

    assert_eq!(program.code, 0);
    assert_eq!(json_of(&program.stdout)["value"], "jev-1.13.0");
    let warnings: Vec<Value> = program.stderr.lines().map(json_of).collect();
    assert_eq!(
        warnings.len(),
        2,
        "every stderr line is a JSON object: {}",
        program.stderr
    );
    assert_eq!(warnings[1]["warning"]["code"], "config_unknown_key");
    assert_eq!(warnings[1]["warning"]["hint"], "did you mean `model`?");

    assert_eq!(
        (quiet.code, quiet.stderr.as_str()),
        (0, ""),
        "--quiet suppresses warnings"
    );
}

#[test]
fn a_broken_configuration_stops_only_the_commands_that_need_it() {
    let sandbox = Sandbox::with_config("broken", "[profiles.default\nmodel = ");

    let version = run(sandbox.jev().arg("version"));
    let path = run(sandbox.jev().args(["config", "path", "-o", "table"]));
    let list = run(sandbox.jev().args(["config", "list"]));
    let set = run(sandbox.jev().args(["config", "set", "model", "x"]));

    assert_eq!(version.code, 0, "{}", version.stderr);
    assert_eq!(
        (path.code, path.stdout.trim()),
        (0, sandbox.file().to_str().unwrap())
    );
    for failed in [list, set] {
        assert_eq!((failed.code, failed.stdout.as_str()), (2, ""));
        let error = &json_of(&failed.stderr)["error"];
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("is not valid TOML"),
            "{error}"
        );
        assert!(
            error["hint"].as_str().unwrap().contains("by hand"),
            "{error}"
        );
    }
    assert_eq!(
        sandbox.text(),
        "[profiles.default\nmodel = ",
        "a broken file is never overwritten"
    );
}

#[test]
fn config_path_reports_the_directory_in_use() {
    let sandbox = Sandbox::new("path");

    let location = json_of(&ok(sandbox.jev().args(["config", "path"])));

    assert_eq!(location["config_dir"], sandbox.0.to_str().unwrap());
    assert_eq!(location["config_file"], sandbox.file().to_str().unwrap());
    assert_eq!(location["exists"], false);
}

#[test]
fn profiles_are_created_selected_used_and_deleted() {
    let sandbox = Sandbox::new("profiles");

    ok(sandbox.jev().args([
        "profile",
        "create",
        "staging",
        "--base-url",
        "http://127.0.0.1:4010",
        "--model",
        "jev-preview",
        "--timeout",
        "5",
    ]));
    ok(sandbox.jev().args(["profile", "create", "work"]));
    ok(sandbox
        .jev()
        .args(["config", "set", "model", "jev-1.13.0", "--profile", "work"]));

    // Selected for one command by flag or by environment variable.
    assert_eq!(
        ok(sandbox.jev().args([
            "config",
            "get",
            "model",
            "-o",
            "table",
            "--profile",
            "staging"
        ])),
        "jev-preview\n"
    );
    assert_eq!(
        ok(sandbox
            .jev()
            .args(["config", "get", "model", "-o", "table"])
            .env("JEV_PROFILE", "work")),
        "jev-1.13.0\n"
    );
    assert_eq!(
        ok(sandbox
            .jev()
            .args(["config", "get", "model", "-o", "table"])),
        "jev-latest\n"
    );

    // Made the default for every command.
    assert_eq!(
        json_of(&ok(sandbox.jev().args(["profile", "use", "staging"]))),
        json!({ "profile": "staging", "action": "active" })
    );
    assert_eq!(
        ok(sandbox
            .jev()
            .args(["config", "get", "timeout", "-o", "table"])),
        "5s\n"
    );
    let listing = json_of(&ok(sandbox.jev().args(["config", "list"])));
    assert_eq!(
        listing["profile"],
        json!({
            "key": "profile",
            "value": "staging",
            "source": "config",
            "origin": "active_profile",
            "description": "the selected profile"
        })
    );

    let profiles = json_of(&ok(sandbox.jev().args(["profile", "list"])));
    let summary: Vec<(&str, bool)> = profiles["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["name"].as_str().unwrap(), p["active"].as_bool().unwrap()))
        .collect();
    assert_eq!(
        summary,
        [("default", false), ("staging", true), ("work", false)]
    );
    assert_eq!(
        profiles["profiles"][1]["settings"],
        json!({ "base_url": "http://127.0.0.1:4010", "model": "jev-preview", "timeout": "5s" })
    );
    let table = ok(sandbox.jev().args(["profile", "list", "-o", "table"]));
    assert!(
        table.contains("*  staging  base_url=http://127.0.0.1:4010  model=jev-preview  timeout=5s"),
        "{table}"
    );
    assert!(table.contains("   default  (defaults)"), "{table}");

    // Deleting the active profile falls back to `default`.
    ok(sandbox.jev().args(["profile", "delete", "staging"]));
    assert_eq!(
        ok(sandbox
            .jev()
            .args(["config", "get", "model", "-o", "table"])),
        "jev-latest\n"
    );
    assert!(!sandbox.text().contains("staging"), "{}", sandbox.text());
}

#[test]
fn profile_mistakes_are_usage_errors_with_a_next_step() {
    let sandbox = Sandbox::new("profile-errors");
    ok(sandbox.jev().args(["profile", "create", "staging"]));
    let cases: [(&[&str], &str, &str); 6] = [
        (
            &["profile", "create", "staging"],
            "profile `staging` already exists",
            "jev config set",
        ),
        (
            &["profile", "create", "default"],
            "profile `default` already exists",
            "jev config set",
        ),
        (
            &["profile", "create", "bad name"],
            "`bad name` is not a valid profile name",
            "letters, digits",
        ),
        (
            &["profile", "use", "stagin"],
            "there is no profile `stagin`",
            "did you mean `staging`?",
        ),
        (
            &["profile", "delete", "nope"],
            "there is no profile `nope`",
            "the profiles are: default, staging",
        ),
        (
            &["config", "list", "--profile", "stagin"],
            "there is no profile `stagin` (selected by --profile)",
            "did you mean `staging`?",
        ),
    ];

    for (arguments, message, hint) in cases {
        let run = run(sandbox.jev().args(arguments));

        assert_eq!((run.code, run.stdout.as_str()), (2, ""), "{arguments:?}");
        let error = &json_of(&run.stderr)["error"];
        assert_eq!(error["message"], message, "{arguments:?}");
        assert!(
            error["hint"].as_str().unwrap().contains(hint),
            "{arguments:?}: {error}"
        );
    }
    let by_env = run(sandbox
        .jev()
        .args(["version"])
        .env("JEV_PROFILE", "missing"));
    assert_eq!(
        by_env.code, 0,
        "a command that needs no settings is not stopped by a bad profile"
    );
}

#[test]
fn the_profile_can_set_the_output_format_and_a_flag_still_wins() {
    let sandbox = Sandbox::with_config("format", "[profiles.default]\noutput = \"yaml\"\n");

    let from_profile = ok(sandbox.jev().arg("version"));
    let from_env = ok(sandbox.jev().arg("version").env("JEV_OUTPUT", "jsonl"));
    let from_flag = ok(sandbox.jev().args(["version", "-o", "json"]));

    assert!(from_profile.starts_with("version: "), "{from_profile}");
    assert_eq!(from_env.lines().count(), 1, "{from_env}");
    assert!(from_flag.starts_with("{\n"), "{from_flag}");
}

#[test]
fn concurrent_writers_never_corrupt_the_file_or_lose_a_change() {
    let sandbox = Sandbox::new("concurrent");
    let writers = 12;

    let children: Vec<std::process::Child> = (0..writers)
        .map(|index| {
            let mut command = std::process::Command::new(assert_cmd::cargo::cargo_bin("jev"));
            command
                .args([
                    "profile",
                    "create",
                    &format!("p{index}"),
                    "--model",
                    &format!("model-{index}"),
                ])
                .env("JEV_CONFIG_DIR", &sandbox.0)
                .env_remove("JEV_PROFILE")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped());
            command.spawn().unwrap()
        })
        .collect();
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let profiles = json_of(&ok(sandbox.jev().args(["profile", "list"])));
    let names: Vec<&str> = profiles["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names.len(),
        writers + 1,
        "every writer's profile survived: {names:?}"
    );
    for index in 0..writers {
        let model = ok(sandbox.jev().args([
            "config",
            "get",
            "model",
            "-o",
            "table",
            "--profile",
            &format!("p{index}"),
        ]));
        assert_eq!(model, format!("model-{index}\n"));
    }
    let leftovers: Vec<String> = fs::read_dir(&sandbox.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| {
            Path::new(name)
                .extension()
                .is_some_and(|extension| extension == "tmp")
        })
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}
