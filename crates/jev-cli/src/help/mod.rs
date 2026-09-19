//! The help standard: what every command's `--help`, and `jev spec`, say beyond its flags.
//!
//! A command's one-line purpose and its arguments live in `cli.rs`, as `clap` definitions. The
//! rest of its help lives here, as a [`Doc`]: when to use it and when to use a sibling instead,
//! its input, its output, the exit codes it can return, and 2 to 4 examples. [`apply`] attaches
//! the docs to the `clap` tree, so `--help` and `jev spec` read the same text and cannot drift.
//!
//! A test (`lint`) walks the tree and fails, saying what to add, when a command lacks any part.

mod docs;
#[cfg(test)]
mod lint;

use std::fmt::Write as _;

use clap::builder::styling::Style;
use serde::Serialize;

use crate::exit::Exit;

pub(crate) use docs::{DOCS, ROOT_EXAMPLES};

/// The help of one command, beyond its purpose and its flags.
#[derive(Debug)]
pub(crate) struct Doc {
    /// The command's path below `jev`, such as `noul` or `auth login`.
    pub(crate) path: &'static str,
    /// When to use the command, and which sibling to use instead in the other cases.
    pub(crate) when: &'static str,
    /// What the command reads, and where the state comes from.
    pub(crate) input: &'static str,
    /// What the command prints on stdout.
    pub(crate) output: &'static str,
    /// The exit codes it can return, in ascending order. Exit 1, a bug, is left out: any command
    /// can return it.
    pub(crate) exit_codes: &'static [ExitCode],
    /// 2 to 4 copy-pasteable examples, at least one of them machine-readable.
    pub(crate) examples: &'static [Example],
}

/// An exit code a command can return, and what it means for that command.
#[derive(Debug)]
pub(crate) struct ExitCode {
    pub(crate) exit: Exit,
    meaning: Option<&'static str>,
}

impl ExitCode {
    /// What the code means for this command.
    pub(crate) const fn meaning(&self) -> &'static str {
        match self.meaning {
            Some(meaning) => meaning,
            None => self.exit.meaning(),
        }
    }
}

/// An exit code with the meaning it has for every command.
const fn code(exit: Exit) -> ExitCode {
    ExitCode {
        exit,
        meaning: None,
    }
}

/// An exit code with what it means for one command.
const fn code_as(exit: Exit, meaning: &'static str) -> ExitCode {
    ExitCode {
        exit,
        meaning: Some(meaning),
    }
}

/// A copy-pasteable example.
#[derive(Debug, Serialize)]
pub(crate) struct Example {
    /// What the example does, in a few words.
    pub(crate) description: &'static str,
    /// The shell command, possibly several lines joined by `\`.
    pub(crate) command: &'static str,
}

/// The doc of the command at `path`, such as `auth login`.
pub(crate) fn find(path: &str) -> Option<&'static Doc> {
    DOCS.iter().find(|doc| doc.path == path)
}

/// Whether a command is still a placeholder: its only argument is the catch-all of
/// [`crate::cli::Pending`]. A placeholder needs no doc until it is given its real arguments.
pub(crate) fn is_pending(command: &clap::Command) -> bool {
    command
        .get_arguments()
        .any(|argument| argument.get_id() == PENDING_ARGUMENT)
}

/// The id of the one argument of [`crate::cli::Pending`].
const PENDING_ARGUMENT: &str = "rest";

/// Attaches the docs to the command tree: the sections before the flags in `--help`, and the exit
/// codes and examples after them in both `-h` and `--help`.
pub(crate) fn apply(command: clap::Command) -> clap::Command {
    let header = *command.get_styles().get_header();
    let root_after = render_after(&root_exit_codes(), ROOT_EXAMPLES, header);
    let command = command
        .after_help(root_after.clone())
        .after_long_help(root_after);
    apply_below(command, "", header)
}

fn apply_below(mut command: clap::Command, path: &str, header: Style) -> clap::Command {
    if let Some(doc) = find(path) {
        let about = command
            .get_about()
            .map(ToString::to_string)
            .unwrap_or_default();
        let after = render_after(doc.exit_codes, doc.examples, header);
        command = command
            .long_about(render_before(&about, doc, header))
            .after_help(after.clone())
            .after_long_help(after);
    }
    let names: Vec<String> = command
        .get_subcommands()
        .map(|sub| sub.get_name().to_owned())
        .collect();
    for name in names {
        let below = join(path, &name);
        command = command.mut_subcommand(&name, |sub| apply_below(sub, &below, header));
    }
    command
}

/// `auth` and `login` make `auth login`.
pub(crate) fn join(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_owned()
    } else {
        format!("{path} {name}")
    }
}

/// Every exit code, for the help of `jev` itself.
fn root_exit_codes() -> Vec<ExitCode> {
    Exit::ALL.into_iter().map(code).collect()
}

fn render_before(about: &str, doc: &Doc, header: Style) -> String {
    let mut text = about.to_owned();
    for (title, body) in [
        ("When to use", doc.when),
        ("Input", doc.input),
        ("Output", doc.output),
    ] {
        let _ = write!(text, "\n\n{header}{title}:{header:#}\n{body}");
    }
    text
}

fn render_after(exit_codes: &[ExitCode], examples: &[Example], header: Style) -> String {
    let mut text = format!("{header}Exit codes:{header:#}\n");
    for exit_code in exit_codes {
        let _ = writeln!(
            text,
            "  {:<4} {}",
            exit_code.exit.code(),
            exit_code.meaning()
        );
    }
    let _ = write!(text, "\n{header}Examples:{header:#}");
    for example in examples {
        let _ = write!(text, "\n  # {}", example.description);
        for line in example.command.lines() {
            let _ = write!(text, "\n  {line}");
        }
        text.push('\n');
    }
    text.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use clap::{Args, Command};

    use super::is_pending;
    use crate::cli::Pending;

    #[test]
    fn a_placeholder_command_is_recognised_by_its_catch_all_argument() {
        let pending = Pending::augment_args(Command::new("later"));
        let real = Command::new("now").arg(clap::Arg::new("file").long("file"));

        assert!(is_pending(&pending));
        assert!(!is_pending(&real));
    }
}
