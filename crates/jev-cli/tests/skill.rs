//! The agent skill in `skills/jev-cli/SKILL.md` held against the real binary, so that it cannot
//! drift: every command and flag it names exists in `jev spec` (and is implemented), its exit-code
//! table is the spec's, the MCP tools it lists are the ones `jev mcp serve` offers, and every
//! request file it shows passes `jev validate --strict`.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use assert_cmd::Command;
use serde_json::{Value, json};

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

/// What `jev spec` says exists: implemented command paths with their flags, and global flags.
struct Contract {
    commands: BTreeMap<String, BTreeSet<String>>,
    pending: BTreeSet<String>,
    global: BTreeSet<String>,
}

impl Contract {
    fn load() -> Self {
        let spec = spec();
        let flags = |entries: &Value| -> BTreeSet<String> {
            let mut names = BTreeSet::new();
            for flag in entries.as_array().unwrap() {
                names.insert(flag["name"].as_str().unwrap().to_owned());
                if let Some(short) = flag["short"].as_str() {
                    names.insert(short.to_owned());
                }
            }
            names
        };
        let mut global = flags(&spec["global_flags"]);
        global.extend(["--help", "-h", "--version", "-V"].map(str::to_owned));
        let mut commands = BTreeMap::new();
        let mut pending = BTreeSet::new();
        for command in spec["commands"].as_array().unwrap() {
            let path = command["path"].as_str().unwrap().to_owned();
            if command["implemented"].as_bool().unwrap() {
                commands.insert(path, flags(&command["flags"]));
            } else {
                pending.insert(path);
            }
        }
        Self {
            commands,
            pending,
            global,
        }
    }

    /// The command a list of words names: the longest implemented path, or a group such as
    /// `schema` that some implemented path starts with.
    fn resolve(&self, words: &[String]) -> Result<String, String> {
        let names: Vec<&str> = words
            .iter()
            .take_while(|word| !word.starts_with('-'))
            .map(String::as_str)
            .collect();
        for length in (1..=names.len()).rev() {
            let candidate = names[..length].join(" ");
            if self.pending.contains(&candidate) {
                return Err(format!("`jev {candidate}` is not implemented yet"));
            }
            if self.commands.contains_key(&candidate)
                || self
                    .commands
                    .keys()
                    .any(|path| path.starts_with(&format!("{candidate} ")))
            {
                return Ok(candidate);
            }
        }
        match names.first() {
            Some(name) => Err(format!("`jev {name}` is not a command")),
            None => Ok(String::new()),
        }
    }

    fn has_flag(&self, command: &str, flag: &str) -> bool {
        self.global.contains(flag)
            || self
                .commands
                .get(command)
                .is_some_and(|flags| flags.contains(flag))
    }

    fn any_has_flag(&self, flag: &str) -> bool {
        self.global.contains(flag) || self.commands.values().any(|flags| flags.contains(flag))
    }
}

/// A word of a shell command, and whether it was quoted (a quoted word is never a flag).
#[derive(Debug)]
struct Word {
    text: String,
    quoted: bool,
}

/// Splits shell text into simple commands: words, split at `|`, `||`, `&&`, `;` and `;;`, with
/// comments and redirection targets dropped and quotes honoured. Enough for documentation.
fn commands(text: &str) -> Vec<Vec<Word>> {
    let mut commands = vec![Vec::new()];
    let mut chars = text.chars().peekable();
    let mut skip_next = false;
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        if matches!(c, '|' | '&' | ';') {
            while chars.peek().is_some_and(|c| matches!(c, '|' | '&' | ';')) {
                chars.next();
            }
            commands.push(Vec::new());
            continue;
        }
        if matches!(c, '>' | '<') {
            while chars.peek().is_some_and(|c| matches!(c, '>' | '<')) {
                chars.next();
            }
            skip_next = true;
            continue;
        }
        if c == '#' {
            break;
        }
        let mut word = String::new();
        let mut quoted = false;
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() || matches!(c, '|' | '&' | ';' | '>' | '<') {
                break;
            }
            chars.next();
            if c == '\'' || c == '"' {
                quoted = true;
                for inner in chars.by_ref() {
                    if inner == c {
                        break;
                    }
                    word.push(inner);
                }
            } else {
                word.push(c);
            }
        }
        if std::mem::take(&mut skip_next) {
            continue;
        }
        commands
            .last_mut()
            .unwrap()
            .push(Word { text: word, quoted });
    }
    commands.retain(|command| !command.is_empty());
    commands
}

/// The `jev` invocations in a simple command: one that starts with `jev` (after shell keywords),
/// and one after a `--` that hands the rest of the line to another program, as in
/// `claude mcp add jev -- jev mcp serve`.
fn invocations(command: &[Word]) -> Vec<&[Word]> {
    let mut found = Vec::new();
    let start = command
        .iter()
        .position(|word| !matches!(word.text.as_str(), "if" | "then" | "!" | "else" | "do"))
        .unwrap_or(command.len());
    if command.get(start).is_some_and(|word| word.text == "jev") {
        found.push(&command[start + 1..]);
    } else if let Some(separator) = command.iter().position(|word| word.text == "--")
        && command
            .get(separator + 1)
            .is_some_and(|word| word.text == "jev")
    {
        found.push(&command[separator + 2..]);
    }
    found
}

/// The flags in a list of words: `--name` (before any `=`), and each letter of `-abc`.
fn flags(words: &[Word]) -> Vec<String> {
    let mut flags = Vec::new();
    for word in words.iter().filter(|word| !word.quoted) {
        let text = word.text.as_str();
        if let Some(long) = text.strip_prefix("--") {
            if !long.is_empty() {
                flags.push(format!("--{}", long.split('=').next().unwrap()));
            }
        } else if let Some(short) = text.strip_prefix('-')
            && !short.is_empty()
            && short.chars().all(|c| c.is_ascii_alphabetic())
        {
            flags.extend(short.chars().map(|c| format!("-{c}")));
        }
    }
    flags
}

/// A piece of code in the skill: a line of a shell block, or an inline code span.
struct Code {
    line: usize,
    text: String,
    /// In a table row such as "| `jev choice` | `--expect` |", the command named by an earlier
    /// cell, which a bare flag after it belongs to.
    context: Option<String>,
    inline: bool,
}

/// Every piece of shell in the skill, and every fenced block by language.
fn code(skill: &str) -> (Vec<Code>, Vec<(String, String)>) {
    let mut pieces = Vec::new();
    let mut blocks = Vec::new();
    let mut fence: Option<(String, String)> = None;
    let mut continued = String::new();
    for (index, line) in skill.lines().enumerate() {
        let number = index + 1;
        if let Some(language) = line.trim_start().strip_prefix("```") {
            match fence.take() {
                Some(block) => blocks.push(block),
                None => fence = Some((language.trim().to_owned(), String::new())),
            }
            continue;
        }
        if let Some((language, body)) = &mut fence {
            body.push_str(line);
            body.push('\n');
            if matches!(language.as_str(), "bash" | "sh" | "shell") {
                if let Some(head) = line.strip_suffix('\\') {
                    continued.push_str(head);
                    continued.push(' ');
                } else {
                    continued.push_str(line);
                    pieces.push(Code {
                        line: number,
                        text: std::mem::take(&mut continued),
                        context: None,
                        inline: false,
                    });
                }
            }
            continue;
        }
        let spans: Vec<&str> = line.split('`').collect();
        assert!(
            spans.len() % 2 == 1,
            "SKILL.md:{number}: a code span runs onto the next line; keep each on one line"
        );
        let mut context = None;
        for span in spans.iter().skip(1).step_by(2) {
            pieces.push(Code {
                line: number,
                text: (*span).to_owned(),
                context: context.clone(),
                inline: true,
            });
            if line.starts_with('|')
                && let Some(rest) = span.strip_prefix("jev ")
            {
                context = Some(rest.split_whitespace().collect::<Vec<_>>().join(" "));
            }
        }
    }
    assert!(fence.is_none(), "SKILL.md: a code block is never closed");
    (pieces, blocks)
}

#[test]
fn every_command_and_flag_the_skill_names_exists_in_the_spec() {
    let contract = Contract::load();
    let (pieces, _) = code(&skill());
    let mut problems = Vec::new();
    let mut checked = 0;
    for piece in &pieces {
        for command in commands(&piece.text) {
            let invoked = invocations(&command);
            for words in &invoked {
                checked += 1;
                let path = match contract.resolve(
                    &words
                        .iter()
                        .map(|word| word.text.clone())
                        .collect::<Vec<_>>(),
                ) {
                    Ok(path) => path,
                    Err(problem) => {
                        problems.push(format!("SKILL.md:{}: {problem}", piece.line));
                        continue;
                    }
                };
                for flag in flags(words) {
                    if !contract.has_flag(&path, &flag) {
                        problems.push(format!(
                            "SKILL.md:{}: `jev {path}` has no flag {flag}",
                            piece.line
                        ));
                    }
                }
            }
            // A span that is only flags, such as `--fail-under`, belongs to the command named
            // earlier on its line, or else to some command.
            if invoked.is_empty() && piece.inline && command[0].text.starts_with('-') {
                for flag in flags(&command) {
                    let known = match &piece.context {
                        Some(context) => contract
                            .resolve(
                                &context
                                    .split(' ')
                                    .map(str::to_owned)
                                    .collect::<Vec<String>>(),
                            )
                            .is_ok_and(|path| contract.has_flag(&path, &flag)),
                        None => contract.any_has_flag(&flag),
                    };
                    if !known {
                        problems.push(format!(
                            "SKILL.md:{}: no command {} has the flag {flag}",
                            piece.line,
                            piece
                                .context
                                .as_ref()
                                .map_or_else(String::new, |context| format!("`jev {context}`")),
                        ));
                    }
                }
            }
        }
    }
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
    let (_, blocks) = code(&skill());
    let files: Vec<&String> = blocks
        .iter()
        .filter(|(language, _)| matches!(language.as_str(), "yaml" | "json"))
        .map(|(_, body)| body)
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
