//! `jev completion`: the script on stdout as is, and a script each shell can parse.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command as Process;

use assert_cmd::Command;
use serde_json::Value;

/// `jev`, isolated from the environment of whoever runs the tests.
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
    command.env("JEV_CONFIG_DIR", scratch("no-config"));
    command
}

fn scratch(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("completion")
        .join(name)
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

/// The script for `shell`, checked to be a clean success.
fn script(shell: &str) -> String {
    let run = run(jev().args(["completion", shell]));
    assert_eq!((run.code, run.stderr.as_str()), (0, ""), "{shell}");
    run.stdout
}

#[test]
fn every_shell_gets_its_script_on_stdout_as_is_not_as_json() {
    for (shell, marker) in [
        ("bash", "complete -F _jev"),
        ("zsh", "#compdef jev"),
        ("fish", "complete -c jev"),
        ("powershell", "Register-ArgumentCompleter"),
    ] {
        let script = script(shell);

        assert!(script.contains(marker), "{shell}: {script}");
        assert!(serde_json::from_str::<Value>(&script).is_err(), "{shell}");
    }
}

#[test]
fn output_flags_do_not_wrap_the_script() {
    let plain = script("bash");

    for format in ["json", "yaml", "jsonl", "table"] {
        let run = run(jev().args(["completion", "bash", "-o", format]));
        assert_eq!((run.code, run.stderr.as_str()), (0, ""), "{format}");
        assert_eq!(run.stdout, plain, "{format}");
    }
}

#[test]
fn the_shell_name_is_case_insensitive() {
    assert_eq!(script("PowerShell"), script("powershell"));
}

#[test]
fn a_missing_or_unknown_shell_is_a_usage_error() {
    for arguments in [vec!["completion"], vec!["completion", "tcsh"]] {
        let run = run(jev().args(&arguments).args(["-o", "json"]));

        assert_eq!((run.code, run.stdout.as_str()), (2, ""), "{arguments:?}");
        let error: Value = serde_json::from_str(&run.stderr).unwrap();
        assert_eq!(error["error"]["code"], "usage", "{error}");
    }
}

#[test]
fn it_needs_no_configuration_and_no_key() {
    let broken = scratch("broken-config");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("config.toml"), "not = [valid").unwrap();

    let run = run(jev()
        .args(["completion", "fish"])
        .env("JEV_CONFIG_DIR", &broken));

    assert_eq!((run.code, run.stderr.as_str()), (0, ""));
    assert!(run.stdout.contains("complete -c jev"));
}

/// How to check a script's syntax without running it, per shell.
fn checker(shell: &str, path: &Path) -> Process {
    let path = path.to_str().unwrap();
    match shell {
        "bash" | "zsh" => {
            let mut process = Process::new(shell);
            process.args(["-n", path]);
            process
        }
        "fish" => {
            let mut process = Process::new("fish");
            process.args(["--no-execute", path]);
            process
        }
        "powershell" => {
            let mut process = Process::new("pwsh");
            process.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!(
                    "$errors = $null; \
[System.Management.Automation.Language.Parser]::ParseFile('{path}', [ref]$null, [ref]$errors) \
| Out-Null; if ($errors) {{ $errors | Out-String | Write-Error; exit 1 }}"
                ),
            ]);
            process
        }
        _ => panic!("no checker for {shell}"),
    }
}

/// Each shell parses its script, where the shell is installed. `JEV_TEST_SHELLS` (such as
/// `bash,zsh,fish,powershell`, set in CI) names the shells that must be there, so that a missing
/// one fails instead of being skipped.
#[test]
fn every_script_parses_in_its_shell() {
    let required = std::env::var("JEV_TEST_SHELLS").unwrap_or_default();
    let required: Vec<&str> = required
        .split(',')
        .map(str::trim)
        .filter(|shell| !shell.is_empty())
        .collect();
    let directory = scratch("scripts");
    std::fs::create_dir_all(&directory).unwrap();

    for shell in ["bash", "zsh", "fish", "powershell"] {
        let path = directory.join(format!("jev.{shell}"));
        std::fs::write(&path, script(shell)).unwrap();

        match checker(shell, &path).output() {
            Ok(output) => assert!(
                output.status.success(),
                "{shell} cannot parse its completion script:\n{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                assert!(
                    !required.contains(&shell),
                    "{shell} is required by JEV_TEST_SHELLS but is not installed"
                );
                eprintln!("skipped: {shell} is not installed");
            }
            Err(error) => panic!("could not run the {shell} checker: {error}"),
        }
    }
}
