//! The Markdown command reference published as `docs/commands.md`.
//!
//! It is rendered from [`CommandTree`], the same document `jev spec` prints, which is built from
//! the command tree `jev` parses with and the help in [`crate::help`]. So the reference cannot say
//! anything `--help` does not, and a flag that changes without the file being regenerated fails a
//! test (`tests/docs.rs`), which names `make reference`.
//!
//! The version of `jev` is deliberately left out: a release bumps it, and a file that went stale
//! on every release would fail the release pull request.

use std::fmt::Write as _;

use serde_json::Value;

use crate::commands::spec::{Argument, CommandSpec, CommandTree, Flag};
use crate::help::Example;

/// The whole reference, as the file's contents.
pub(crate) fn markdown() -> String {
    let tree = CommandTree::new();
    let mut out = String::new();

    let _ = writeln!(
        out,
        "<!-- Generated from the `jev` command tree by `make reference`. Do not edit by hand. -->\n"
    );
    let _ = writeln!(out, "# jev command reference\n");
    let _ = writeln!(out, "{}\n", tree.about);
    let _ = writeln!(out, "{}\n", tree.description);
    let _ = writeln!(
        out,
        "This page is generated from the command tree `jev` parses with, so it says what \
`jev --help` and `jev spec` say. `jev spec` prints the same content as JSON, for tooling.\n"
    );

    let _ = writeln!(out, "## Examples\n");
    examples(&mut out, tree.examples);

    let _ = writeln!(out, "## Exit codes\n");
    let _ = writeln!(
        out,
        "The whole contract. Each command lists the codes it can return; exit 1 is left out \
below, because any command can return it.\n"
    );
    let _ = writeln!(out, "| Code | Name | Meaning |");
    let _ = writeln!(out, "| ---: | --- | --- |");
    for exit_code in &tree.exit_codes {
        let _ = writeln!(
            out,
            "| {} | `{}` | {} |",
            exit_code.code,
            exit_code.name,
            cell(exit_code.meaning)
        );
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Global flags\n");
    let _ = writeln!(
        out,
        "Accepted by every command, before or after its name.\n"
    );
    flags(&mut out, &tree.global_flags);

    let _ = writeln!(out, "## Commands\n");
    let _ = writeln!(out, "| Command | What it does |");
    let _ = writeln!(out, "| --- | --- |");
    for command in &tree.commands {
        let _ = writeln!(
            out,
            "| [`{name} {path}`](#{anchor}) | {about} |",
            name = tree.name,
            path = command.path,
            anchor = anchor(&tree.name, &command.path),
            about = cell(&command.about),
        );
    }
    let _ = writeln!(out);

    for command in &tree.commands {
        section(&mut out, &tree.name, command);
    }
    // Every section ends with a blank line; a file ends with exactly one newline.
    out.truncate(out.trim_end().len());
    out.push('\n');
    out
}

/// One command: everything its `--help` says.
fn section(out: &mut String, name: &str, command: &CommandSpec) {
    let _ = writeln!(out, "### {name} {}\n", command.path);
    let _ = writeln!(out, "{}\n", command.about);
    if !command.implemented {
        let _ = writeln!(
            out,
            "> Not implemented yet: the command answers with the issue that tracks it.\n"
        );
    }
    let _ = writeln!(out, "```text\n{}\n```\n", command.usage);
    for (title, body) in [
        ("When to use", command.when_to_use),
        ("Input", command.input),
        ("Output", command.output),
    ] {
        if let Some(body) = body {
            let _ = writeln!(out, "**{title}.** {body}\n");
        }
    }

    if !command.arguments.is_empty() {
        let _ = writeln!(out, "**Arguments**\n");
        let _ = writeln!(out, "| Argument | Value | What it is |");
        let _ = writeln!(out, "| --- | --- | --- |");
        for argument in &command.arguments {
            let _ = writeln!(
                out,
                "| `{}` | {} | {} |",
                argument.name,
                value(argument),
                cell(&argument.help)
            );
        }
        let _ = writeln!(out);
    }

    if !command.flags.is_empty() {
        let _ = writeln!(out, "**Flags**\n");
        flags(out, &command.flags);
    }

    if !command.exit_codes.is_empty() {
        let _ = writeln!(out, "**Exit codes**\n");
        let _ = writeln!(out, "| Code | Meaning |");
        let _ = writeln!(out, "| ---: | --- |");
        for exit_code in &command.exit_codes {
            let _ = writeln!(out, "| {} | {} |", exit_code.code, cell(exit_code.meaning));
        }
        let _ = writeln!(out);
    }

    if !command.examples.is_empty() {
        let _ = writeln!(out, "**Examples**\n");
        examples(out, command.examples);
    }
}

/// The flags of one command, or the global flags, in the order and the groups of `--help`.
fn flags(out: &mut String, flags: &[Flag]) {
    let mut heading: Option<Option<&str>> = None;
    for flag in flags {
        let group = flag.heading.as_deref();
        if heading != Some(group) {
            if heading.is_some() {
                let _ = writeln!(out);
            }
            heading = Some(group);
            let _ = writeln!(out, "_{}_\n", group.unwrap_or("Options"));
            let _ = writeln!(
                out,
                "| Flag | Value | Default | Environment | Setting | What it does |"
            );
            let _ = writeln!(out, "| --- | --- | --- | --- | --- | --- |");
        }
        let _ = writeln!(
            out,
            "| `{spelling}` | {value} | {default} | {env} | {setting} | {help} |",
            spelling = spelling(flag),
            value = value(flag),
            default = flag.default.as_ref().map_or_else(String::new, literal),
            env = flag
                .env
                .as_deref()
                .map_or_else(String::new, |env| format!("`{env}`")),
            setting = flag
                .setting
                .map_or_else(String::new, |setting| format!("`{setting}`")),
            help = cell(&without_annotations(&flag.help)),
        );
    }
    let _ = writeln!(out);
}

fn examples(out: &mut String, examples: &[Example]) {
    let _ = writeln!(out, "```bash");
    for (index, example) in examples.iter().enumerate() {
        if index > 0 {
            let _ = writeln!(out);
        }
        let _ = writeln!(out, "# {}", example.description);
        let _ = writeln!(out, "{}", example.command);
    }
    let _ = writeln!(out, "```\n");
}

/// How a flag is written on a command line, such as `-o, --output <FORMAT>`.
fn spelling(flag: &Flag) -> String {
    let mut text = String::new();
    if let Some(short) = &flag.short {
        let _ = write!(text, "{short}, ");
    }
    text.push_str(&flag.name);
    if let Some(value_name) = &flag.value_name {
        let _ = write!(text, " <{value_name}>");
    }
    text
}

/// What a flag or an argument takes: its type, the values of an enum, and whether it is required
/// or may be repeated.
fn value(of: &impl Typed) -> String {
    let mut text = match of.values() {
        Some(values) if !values.is_empty() => values
            .iter()
            .map(|value| format!("`{value}`"))
            .collect::<Vec<_>>()
            .join(", "),
        _ if of.kind() == "boolean" => "flag".to_owned(),
        _ => of.kind().to_owned(),
    };
    if of.required() {
        text.push_str(", required");
    }
    if of.multiple() {
        text.push_str(", repeatable");
    }
    text
}

/// What [`value`] needs of a flag or an argument.
trait Typed {
    fn kind(&self) -> &str;
    fn required(&self) -> bool;
    fn multiple(&self) -> bool;
    fn values(&self) -> Option<Vec<&str>> {
        None
    }
}

impl Typed for Flag {
    fn kind(&self) -> &str {
        self.kind
    }
    fn required(&self) -> bool {
        self.required
    }
    fn multiple(&self) -> bool {
        self.multiple
    }
    fn values(&self) -> Option<Vec<&str>> {
        self.values
            .as_ref()
            .map(|values| values.iter().map(|value| value.value.as_str()).collect())
    }
}

impl Typed for Argument {
    fn kind(&self) -> &str {
        self.kind
    }
    fn required(&self) -> bool {
        self.required
    }
    fn multiple(&self) -> bool {
        self.multiple
    }
}

/// A default value as a reader would type it.
fn literal(value: &Value) -> String {
    match value {
        Value::String(text) => format!("`{text}`"),
        other => format!("`{other}`"),
    }
}

/// Help text without the `[default: ...]` and `[env: ...]` notes `clap` appends to it: the table
/// has a column for each.
fn without_annotations(help: &str) -> String {
    let mut text = help.trim_end();
    loop {
        let Some(open) = text
            .strip_suffix(']')
            .and_then(|head| head.rfind('['))
            .filter(|open| {
                let note = text.get(open + 1..).unwrap_or_default();
                note.starts_with("default:") || note.starts_with("env:")
            })
        else {
            return text.to_owned();
        };
        text = text.get(..open).unwrap_or_default().trim_end();
    }
}

/// Help text inside a table cell: one line, with the column separator escaped.
fn cell(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "\\|")
}

/// The GitHub anchor of a command's section, such as `jev-auth-login`.
fn anchor(name: &str, path: &str) -> String {
    format!("{name} {path}").replace(' ', "-")
}

#[cfg(test)]
mod tests {
    use super::markdown;

    #[test]
    fn the_reference_covers_every_command_with_its_help() {
        let reference = markdown();

        assert!(reference.starts_with("<!-- Generated"), "{reference}");
        assert!(reference.contains("### jev noul\n"));
        assert!(reference.contains("### jev auth login\n"));
        assert!(reference.contains("[`jev noul`](#jev-noul)"));
        // The exit-code contract, and `noul`'s own list.
        assert!(reference.contains("| 10 | `gate_false` |"));
        assert!(reference.contains("`--fail-under <P>`"));
        // A flag's group, its default and the variable behind it.
        assert!(reference.contains("_Gating_"));
        assert!(reference.contains("`-o, --output <FORMAT>`"));
        assert!(reference.contains("`TYPESAFE_DEFAULT_MODEL`"));
        assert!(
            !reference.contains(env!("CARGO_PKG_VERSION")),
            "the version would make the file stale on every release"
        );
        assert!(!reference.contains('\u{1b}'), "the help kept its styling");
    }

    #[test]
    fn a_table_cell_stays_on_one_line_and_keeps_the_separator_out() {
        assert_eq!(super::cell("a\n  b | c"), "a b \\| c");
    }

    #[test]
    fn a_flags_help_loses_only_the_notes_the_table_has_columns_for() {
        let without = super::without_annotations;

        assert_eq!(
            without("Output format [default: table] [env: X]"),
            "Output format"
        );
        assert_eq!(
            without("Time allowed, e.g. `2m` [default: 30s]"),
            "Time allowed, e.g. `2m`"
        );
        assert_eq!(
            without("Inside `LO,HI`, e.g. [0.4,0.6]"),
            "Inside `LO,HI`, e.g. [0.4,0.6]"
        );
    }
}
