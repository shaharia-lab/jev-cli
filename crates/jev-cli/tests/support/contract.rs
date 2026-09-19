//! Holding a Markdown document against the real binary: what `jev spec` says exists, and a small
//! shell reader for the commands a document shows.
//!
//! Shared by `tests/skill.rs` (the agent skill) and `tests/docs.rs` (the README and the pages in
//! `docs/`), so that both check documentation the same way.

// Each test binary uses part of this module; the rest is not dead code in the crate's own sense.
#![allow(dead_code)]
// Clippy's test allowances cover `#[test]` functions only, not the helpers they share.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

/// What `jev spec` says exists: implemented command paths with their flags, and global flags.
pub(crate) struct Contract {
    commands: BTreeMap<String, BTreeSet<String>>,
    pending: BTreeSet<String>,
    global: BTreeSet<String>,
}

impl Contract {
    /// Reads the document `jev spec -o json` prints.
    pub(crate) fn new(spec: &Value) -> Self {
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
    pub(crate) fn resolve(&self, words: &[String]) -> Result<String, String> {
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

    pub(crate) fn has_flag(&self, command: &str, flag: &str) -> bool {
        self.global.contains(flag)
            || self
                .commands
                .get(command)
                .is_some_and(|flags| flags.contains(flag))
    }

    pub(crate) fn any_has_flag(&self, flag: &str) -> bool {
        self.global.contains(flag) || self.commands.values().any(|flags| flags.contains(flag))
    }
}

/// A word of a shell command, and whether it was quoted (a quoted word is never a flag).
#[derive(Debug)]
pub(crate) struct Word {
    pub(crate) text: String,
    quoted: bool,
}

/// Splits shell text into simple commands: words, split at `|`, `||`, `&&`, `;` and `;;`, with
/// comments and redirection targets dropped and quotes honoured. Enough for documentation.
pub(crate) fn commands(text: &str) -> Vec<Vec<Word>> {
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
pub(crate) fn invocations(command: &[Word]) -> Vec<&[Word]> {
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
pub(crate) fn flags(words: &[Word]) -> Vec<String> {
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

/// A piece of code in a document: a line of a shell block, or an inline code span.
pub(crate) struct Code {
    pub(crate) line: usize,
    pub(crate) text: String,
    /// In a table row such as "| `jev choice` | `--expect` |", the command named by an earlier
    /// cell, which a bare flag after it belongs to.
    pub(crate) context: Option<String>,
    pub(crate) inline: bool,
}

/// A fenced code block: its language and its body.
pub(crate) struct Block {
    pub(crate) language: String,
    pub(crate) body: String,
}

/// Every piece of shell in a Markdown document, and every fenced block by language. `name` is the
/// file, for the assertions' messages.
pub(crate) fn code(name: &str, document: &str) -> (Vec<Code>, Vec<Block>) {
    let mut pieces = Vec::new();
    let mut blocks = Vec::new();
    let mut fence: Option<Block> = None;
    let mut continued = String::new();
    for (index, line) in document.lines().enumerate() {
        let number = index + 1;
        if let Some(language) = line.trim_start().strip_prefix("```") {
            match fence.take() {
                Some(block) => blocks.push(block),
                None => {
                    fence = Some(Block {
                        language: language.trim().to_owned(),
                        body: String::new(),
                    });
                }
            }
            continue;
        }
        if let Some(block) = &mut fence {
            block.body.push_str(line);
            block.body.push('\n');
            if matches!(block.language.as_str(), "bash" | "sh" | "shell") {
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
            "{name}:{number}: a code span runs onto the next line; keep each on one line"
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
    assert!(fence.is_none(), "{name}: a code block is never closed");
    (pieces, blocks)
}

/// Every problem in the `jev` command lines of a document: a command or a flag that does not
/// exist, or one that is not implemented yet. `name` is the file, for the messages, and `ignore`
/// lists flags of other programs the document mentions on their own, such as an install script's.
pub(crate) fn problems(
    contract: &Contract,
    name: &str,
    pieces: &[Code],
    ignore: &[&str],
) -> (Vec<String>, usize) {
    let mut problems = Vec::new();
    let mut checked = 0;
    for piece in pieces {
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
                        problems.push(format!("{name}:{}: {problem}", piece.line));
                        continue;
                    }
                };
                for flag in flags(words) {
                    if !contract.has_flag(&path, &flag) {
                        problems.push(format!(
                            "{name}:{}: `jev {path}` has no flag {flag}",
                            piece.line
                        ));
                    }
                }
            }
            // A span that is only flags, such as `--fail-under`, belongs to the command named
            // earlier on its line, or else to some command.
            if invoked.is_empty() && piece.inline && command[0].text.starts_with('-') {
                for flag in flags(&command)
                    .into_iter()
                    .filter(|flag| !ignore.contains(&flag.as_str()))
                {
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
                            "{name}:{}: no command {} has the flag {flag}",
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
    (problems, checked)
}
