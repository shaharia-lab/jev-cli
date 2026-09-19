//! The agent skill in `skills/jev-cli/SKILL.md` held against the real binary, so that it cannot
//! drift: every command and flag it names exists in `jev spec` (and is implemented), its exit-code
//! table is the spec's, the MCP tools it lists are the ones `jev mcp serve` offers, and every
//! request file it shows passes `jev validate --strict`.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[path = "support/contract.rs"]
mod contract;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use assert_cmd::Command;
use serde_json::{Value, json};

use crate::contract::{Contract, code, commands, flags, invocations, problems};

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
    command.timeout(Duration::from_secs(60));
    command
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn skill() -> String {
    std::fs::read_to_string(repository().join("skills/jev-cli/SKILL.md")).unwrap()
}

fn spec() -> Value {
    let output = jev().args(["spec", "-o", "json"]).output().unwrap();
    assert!(output.status.success(), "jev spec failed");
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn every_command_and_flag_the_skill_names_exists_in_the_spec() {
    let contract = Contract::new(&spec());
    let (pieces, _) = code("SKILL.md", &skill());

    let (problems, checked) = problems(&contract, "SKILL.md", &pieces, &[]);
    assert!(
        problems.is_empty(),
        "the skill names what jev does not have:\n{}",
        problems.join("\n")
    );
    assert!(
        checked > 20,
        "only {checked} jev commands found in the skill"
    );
}

#[test]
fn the_skills_exit_code_table_is_the_specs() {
    let spec = spec();
    let expected: BTreeSet<(u64, String)> = spec["exit_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|code| {
            (
                code["code"].as_u64().unwrap(),
                code["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let documented: BTreeSet<(u64, String)> = skill()
        .lines()
        .filter_map(|line| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let code = cells.get(1)?.parse().ok()?;
            let name = cells.get(2)?;
            Some((code, (*name).to_owned()))
        })
        .collect();
    assert_eq!(documented, expected);
}

#[test]
fn the_mcp_tools_the_skill_lists_are_the_servers() {
    let skill = skill().replace('\n', " ");
    let listed: BTreeSet<String> = skill
        .split_once("MCP tools:")
        .unwrap()
        .1
        .split_once('.')
        .unwrap()
        .0
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect();

    let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
    let output = jev()
        .args(["mcp", "serve"])
        .write_stdin(format!("{request}\n"))
        .output()
        .unwrap();
    assert!(output.status.success(), "jev mcp serve failed");
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    let served: BTreeSet<String> = response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(listed, served);
}

#[test]
fn every_request_file_in_the_skill_is_valid() {
    let (_, blocks) = code("SKILL.md", &skill());
    let files: Vec<&String> = blocks
        .iter()
        .filter(|block| matches!(block.language.as_str(), "yaml" | "json"))
        .map(|block| &block.body)
        .collect();
    assert!(!files.is_empty(), "the skill shows no request file");
    for (index, body) in files.into_iter().enumerate() {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("skill-request-{index}"));
        std::fs::write(&path, body).unwrap();
        let output = jev()
            .args([
                "validate",
                "--strict",
                "--input-format",
                "yaml",
                "-o",
                "json",
                "-f",
            ])
            .arg(&path)
            .args([
                "--state",
                "Payouts have failed since Monday and we lose money every hour.",
            ])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "a request file in the skill is not valid:\n{body}\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            output.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn the_skill_installs_as_a_skill_and_as_a_claude_code_plugin() {
    let skill = skill();
    let front = skill
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .unwrap()
        .0;
    assert!(
        front.lines().any(|line| line == "name: jev-cli"),
        "the name must match the directory, as the Agent Skills format requires"
    );
    let description = front.split_once("description:").unwrap().1;
    let description = description.split("\nlicense:").next().unwrap();
    let length = description
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .len();
    assert!(
        (100..=1024).contains(&length),
        "description is {length} characters"
    );
    assert!(
        skill.contains("not affiliated with"),
        "the skill carries the unofficial notice"
    );

    let marketplace: Value = serde_json::from_str(
        &std::fs::read_to_string(repository().join(".claude-plugin/marketplace.json")).unwrap(),
    )
    .unwrap();
    let plugin = &marketplace["plugins"][0];
    let source = repository().join(plugin["source"].as_str().unwrap());
    for directory in plugin["skills"].as_array().unwrap() {
        let directory = source.join(directory.as_str().unwrap());
        assert!(
            directory.join("SKILL.md").is_file(),
            "the plugin's skill directory {} has no SKILL.md",
            directory.display()
        );
    }
}

#[test]
fn the_shell_reader_finds_jev_commands_and_their_flags() {
    let found =
        commands(r#"if ! jev noul "Is --this a flag?" --fail-under 0.7 -o json > out.txt; then"#);
    assert_eq!(found.len(), 2);
    let words = invocations(&found[0])[0];
    assert_eq!(flags(words), ["--fail-under", "-o"]);
    let found = commands("claude mcp add jev -- jev mcp serve --model=jev-1.13.0 # a comment");
    let words = invocations(&found[0])[0];
    assert_eq!(words[0].text, "mcp");
    assert_eq!(flags(words), ["--model"]);
    assert!(invocations(&commands("jq -r '.id' --arg x")[0]).is_empty());
}
