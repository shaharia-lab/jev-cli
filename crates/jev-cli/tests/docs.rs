//! The user documentation held against the real binary, so that it cannot drift: `docs/commands.md`
//! is exactly what this build renders, every command and flag the pages name exists in `jev spec`,
//! their exit-code tables are the spec's, every setting they name is a real one, and the README's
//! quick start runs, end to end, against a mock of the API.

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
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::contract::{Contract, code, problems};

/// A key that must never appear in any output. It is not a real credential.
const SENTINEL_KEY: &str = "sentinel-key-do-not-leak-8b2f";

/// The documentation this test holds to the binary. `docs/commands.md` is generated, and is
/// checked here too, so that a hand-edit of it is caught like any other drift.
const PAGES: [&str; 5] = [
    "README.md",
    "CONTRIBUTING.md",
    "docs/commands.md",
    "docs/exit-codes.md",
    "docs/configuration.md",
];

/// The pages a reader of a release sees, which must stand on their own: the planning documents
/// under `docs/prd/` and `docs/context/` are deleted before the first release.
const USER_PAGES: [&str; 4] = [
    "README.md",
    "docs/commands.md",
    "docs/exit-codes.md",
    "docs/configuration.md",
];

/// Flags of other programs the pages name on their own: the install scripts' options and
/// `npx skills -g`.
const OTHER_PROGRAMS: [&str; 4] = [
    "--install-dir",
    "--no-modify-path",
    "--require-signature",
    "-g",
];

/// The quick start of the README, verbatim. Each line must be in the README, and each runs here
/// against a mock of the API.
const QUICK_START: [&str; 4] = [
    r#"jev noul "Is this message angry?" --state "You charged me twice. Fix it now.""#,
    "jev validate -f triage.yaml --state-file ticket.txt",
    "jev eval -f triage.yaml --state-file ticket.txt",
    "jev eval -f triage.yaml --state-file ticket.txt --field answers.department.choice",
];

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn page(name: &str) -> String {
    std::fs::read_to_string(repository().join(name))
        .unwrap_or_else(|error| panic!("cannot read {name}: {error}"))
}

/// `jev`, isolated from the environment of whoever runs the tests, pointed at `server`, with a
/// scratch directory of its own for the configuration and the files an example reads.
fn jev(server: Option<&MockServer>, directory: &Path) -> Command {
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
    command.env("JEV_CONFIG_DIR", directory.join("config"));
    command.env("TYPESAFE_API_KEY", SENTINEL_KEY);
    // An address nothing listens on, so a test that forgets its mock cannot reach the real API.
    command.env(
        "TYPESAFE_BASE_URL",
        server.map_or_else(|| "http://127.0.0.1:9".to_owned(), MockServer::uri),
    );
    command.current_dir(directory);
    command.timeout(Duration::from_secs(60));
    command
}

/// An empty scratch directory for one test.
fn scratch(test: &str) -> PathBuf {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("docs")
        .join(test);
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn spec(directory: &Path) -> Value {
    let output = jev(None, directory)
        .args(["spec", "-o", "json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "jev spec failed");
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Every fenced block of a page in one of the languages, with the page's name.
fn blocks(languages: &[&str]) -> Vec<(&'static str, String)> {
    let mut found = Vec::new();
    for name in PAGES {
        let (_, fenced) = code(name, &page(name));
        for block in fenced {
            if languages.contains(&block.language.as_str()) {
                found.push((name, block.body));
            }
        }
    }
    found
}

/// The rows of a Markdown table whose first cell is a number: the exit-code tables.
fn exit_code_rows(document: &str) -> Vec<Vec<String>> {
    document
        .lines()
        .filter(|line| line.starts_with('|'))
        .map(|line| {
            line.split('|')
                .map(|cell| cell.trim().trim_matches('`').to_owned())
                .collect::<Vec<_>>()
        })
        .filter(|cells| cells.get(1).is_some_and(|cell| cell.parse::<u8>().is_ok()))
        .collect()
}

#[test]
fn the_committed_command_reference_is_what_this_build_renders() {
    let file = repository().join("docs/commands.md");
    let committed = std::fs::read_to_string(&file)
        .unwrap_or_else(|error| panic!("{}: {error}; run `make reference`", file.display()));

    assert!(
        committed == jev_cli::command_reference(),
        "{} is out of date; run `make reference` and commit the result",
        file.display()
    );
}

#[test]
fn every_command_and_flag_the_documentation_names_exists_in_the_spec() {
    let directory = scratch("spec");
    let contract = Contract::new(&spec(&directory));

    let mut all = Vec::new();
    let mut total = 0;
    for name in PAGES {
        let (pieces, _) = code(name, &page(name));
        let (found, checked) = problems(&contract, name, &pieces, &OTHER_PROGRAMS);
        all.extend(found);
        total += checked;
    }
    assert!(
        all.is_empty(),
        "the documentation names what jev does not have:\n{}",
        all.join("\n")
    );
    assert!(total > 100, "only {total} jev commands found in the pages");
}

#[test]
fn the_exit_code_tables_are_the_specs() {
    let directory = scratch("exit-codes");
    let spec = spec(&directory);
    let contract: BTreeSet<(u8, String)> = spec["exit_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|code| {
            (
                u8::try_from(code["code"].as_u64().unwrap()).unwrap(),
                code["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect();

    // The reference page lists the whole contract, code and name.
    let documented: BTreeSet<(u8, String)> = exit_code_rows(&page("docs/exit-codes.md"))
        .into_iter()
        .filter(|cells| cells.len() > 3)
        .map(|cells| (cells[1].parse().unwrap(), cells[2].clone()))
        .collect();
    assert_eq!(documented, contract);

    // The README's shorter table must at least be right about the codes it does list.
    let codes: BTreeSet<u8> = contract.iter().map(|(code, _)| *code).collect();
    let readme: BTreeSet<u8> = exit_code_rows(&page("README.md"))
        .into_iter()
        .map(|cells| cells[1].parse().unwrap())
        .collect();
    assert!(readme.is_subset(&codes), "{readme:?} against {codes:?}");
    for expected in [0, 2, 3, 10, 11] {
        assert!(
            readme.contains(&expected),
            "the README omits exit {expected}"
        );
    }
}

#[test]
fn every_setting_the_documentation_names_is_a_real_one() {
    let directory = scratch("settings");
    let listed = jev(None, &directory)
        .args(["config", "list", "-o", "json"])
        .output()
        .unwrap();
    assert!(listed.status.success(), "jev config list failed");
    let listed: Value = serde_json::from_slice(&listed.stdout).unwrap();
    let keys: BTreeSet<String> = listed["settings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|setting| setting["key"].as_str().unwrap().to_owned())
        .collect();

    // The key of every `jev config get|set|unset` the pages show.
    let mut checked = 0;
    for name in PAGES {
        let (pieces, _) = code(name, &page(name));
        for piece in pieces {
            for command in crate::contract::commands(&piece.text) {
                let words: Vec<&str> = command.iter().map(|word| word.text.as_str()).collect();
                let ["jev", "config", "get" | "set" | "unset", key, ..] = words.as_slice() else {
                    continue;
                };
                // In `jev config set --profile ci timeout 60s`, which word is the key depends on
                // the flag's value, so only a key written straight after the verb is checked.
                if !key.starts_with('-') {
                    assert!(
                        keys.contains(*key),
                        "{name}:{}: `{key}` is not a setting",
                        piece.line
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked >= 5, "only {checked} settings named in examples");

    // And every setting jev has is documented, so that a new one cannot be forgotten.
    let configuration = page("docs/configuration.md");
    for key in &keys {
        assert!(
            configuration.contains(&format!("`{key}`")),
            "docs/configuration.md does not document the setting `{key}`"
        );
    }
}

#[test]
fn every_request_file_in_the_documentation_is_valid() {
    let directory = scratch("requests");
    // A request file is the block that has questions in it; the others are MCP client
    // configuration, output envelopes and schema snippets.
    let files: Vec<(&str, String)> = blocks(&["yaml", "json"])
        .into_iter()
        .filter(|(_, body)| body.contains("questions:") || body.contains("\"questions\""))
        .collect();
    assert!(!files.is_empty(), "the documentation shows no request file");

    for (index, (name, body)) in files.into_iter().enumerate() {
        let file = directory.join(format!("request-{index}"));
        std::fs::write(&file, &body).unwrap();
        let output = jev(None, &directory)
            .args([
                "validate",
                "--strict",
                "--input-format",
                "yaml",
                "-o",
                "json",
                "-f",
            ])
            .arg(&file)
            .args([
                "--state",
                "Payouts have failed since Monday and we lose money every hour.",
            ])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "a request file in {name} is not valid:\n{body}\n{}",
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
fn the_user_documentation_stands_on_its_own() {
    for name in USER_PAGES {
        let page = page(name);
        for planning in ["docs/prd", "docs/context", "prd/v1.md"] {
            assert!(
                !page.contains(planning),
                "{name} links to {planning}, which is deleted before the first release"
            );
        }
        assert!(
            page.contains("jev"),
            "{name} does not look like documentation for jev"
        );
    }
    assert!(
        page("README.md").contains("not affiliated with"),
        "the README must carry the unofficial notice"
    );
}

#[test]
fn every_link_in_the_documentation_points_at_something() {
    let mut checked = 0;
    for name in PAGES {
        let directory = Path::new(name).parent().unwrap_or(Path::new("")).to_owned();
        for target in links(&page(name)) {
            let target = target.split('#').next().unwrap_or_default();
            if target.is_empty() || target.starts_with("http") || target.starts_with("mailto:") {
                continue;
            }
            let path = repository().join(&directory).join(target);
            assert!(
                path.exists(),
                "{name} links to {target}, which does not exist"
            );
            checked += 1;
        }
    }
    assert!(checked > 20, "only {checked} links found in the pages");
}

/// The target of every Markdown link in a document.
fn links(document: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut rest = document;
    while let Some(open) = rest.find("](") {
        rest = &rest[open + 2..];
        if let Some(close) = rest.find(')') {
            targets.push(rest[..close].to_owned());
            rest = &rest[close..];
        }
    }
    targets
}

/// The quick start of the README, run as written: the same command lines, against a mock of the
/// API, with the request file the README shows.
#[tokio::test(flavor = "multi_thread")]
async fn the_readme_quick_start_works() {
    let readme = page("README.md");
    let directory = scratch("quick-start");

    // The request file the README tells the reader to write, and a state to ask about.
    let (_, fenced) = code("README.md", &readme);
    let triage = fenced
        .iter()
        .find(|block| block.language == "yaml" && block.body.contains("questions:"))
        .expect("the README shows no request file")
        .body
        .clone();
    std::fs::write(directory.join("triage.yaml"), &triage).unwrap();
    std::fs::write(
        directory.join("ticket.txt"),
        "You charged me twice. Fix it now.",
    )
    .unwrap();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-typesafe-request-id", "req_quickstart")
                .set_body_json(json!({
                    "model": "jev-1.13.0",
                    "answers": {
                        "answer": { "type": "noul", "noul": 0.91 },
                        "is_urgent": { "type": "noul", "noul": 0.94 },
                        "department": {
                            "type": "choice",
                            "choice": "billing",
                            "confidence": 0.89,
                            "probabilities": { "billing": 0.93, "technical": 0.07, "other": 0.0 }
                        },
                        "frustration": {
                            "type": "score",
                            "score": 1.2,
                            "confidence": 0.68,
                            "legend": { "0": "Calm", "1": "Frustrated", "2": "Very angry" },
                            "probabilities": { "0": 0.01, "1": 0.78, "2": 0.21 }
                        }
                    },
                    "usage": { "input_tokens": 459, "output_tokens": 73 }
                })),
        )
        .mount(&server)
        .await;

    for line in QUICK_START {
        assert!(
            readme.contains(line),
            "the README no longer shows `{line}`; keep the quick start and this test together"
        );
        let words: Vec<String> = crate::contract::commands(line)[0]
            .iter()
            .skip(1)
            .map(|word| word.text.clone())
            .collect();
        let mut command = jev(Some(&server), &directory);
        command.args(&words);
        let output = tokio::task::spawn_blocking(move || command.output().unwrap())
            .await
            .unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "`{line}`\n{stdout}\n{stderr}"
        );
        assert!(!stdout.is_empty(), "`{line}` printed nothing");
        for stream in [&stdout, &stderr] {
            assert!(!stream.contains(SENTINEL_KEY), "the key leaked: {stream}");
        }
    }

    // The last line of the quick start prints one field and nothing else.
    let mut command = jev(Some(&server), &directory);
    command.args(["eval", "-f", "triage.yaml", "--state-file", "ticket.txt"]);
    command.args(["--field", "answers.department.choice"]);
    let output = tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap();
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "billing\n");
}

/// The gate example of the README and of `docs/exit-codes.md`: exit 10, the answer printed as
/// usual, and nothing on stderr.
#[tokio::test(flavor = "multi_thread")]
async fn a_gate_that_does_not_hold_exits_10_without_an_error() {
    let directory = scratch("gate");
    std::fs::write(directory.join("ticket.txt"), "Thanks, all sorted.").unwrap();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "jev-1.13.0",
            "answers": { "answer": { "type": "noul", "noul": 0.12 } },
            "usage": { "input_tokens": 40, "output_tokens": 7 }
        })))
        .mount(&server)
        .await;

    let mut command = jev(Some(&server), &directory);
    command.args(["noul", "Is this ticket about billing?"]);
    command.args([
        "--state-file",
        "ticket.txt",
        "--fail-under",
        "0.7",
        "-o",
        "json",
    ]);
    let output = tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap();

    assert_eq!(output.status.code(), Some(10));
    let stdout: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(stdout["noul"], 0.12);
    assert_eq!(stdout["gate"]["passed"], false);
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "",
        "exit 10 is not an error, so stderr stays empty"
    );
}
