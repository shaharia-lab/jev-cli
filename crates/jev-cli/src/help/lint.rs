//! The help standard, enforced: every command that has its real arguments must have a complete
//! [`Doc`], and every example must run `jev` as written. Failures say what is missing and where to
//! add it, because the person reading them is usually adding a command.

use super::{Doc, Example, is_pending, join};

/// Where the docs live, for the failure messages.
const DOCS_FILE: &str = "crates/jev-cli/src/help/docs.rs";

/// What an example must contain to count as machine-readable.
const MACHINE_READABLE: [&str; 6] = [
    "-o json",
    "-o jsonl",
    "-o yaml",
    "--output json",
    "--field ",
    "| jq",
];

/// Commands whose stdout is a file of its own kind rather than a result, so that --output and
/// --field do not apply and no example can be machine-readable in the sense above.
const RAW_OUTPUT: [&str; 1] = ["completion"];

/// The longest line of an example. `--help` indents it by 2 and wraps at 100 columns, and a
/// command line broken by wrapping cannot be pasted: continue a long one with `\` instead.
const EXAMPLE_WIDTH: usize = 96;

/// The longest meaning of an exit code, which follows a 7-column code in `--help`.
const MEANING_WIDTH: usize = 92;

/// Every way `tree`, as `cli.rs` defines it, falls short of the standard with these `docs`.
fn problems(tree: &clap::Command, docs: &[Doc], root_examples: &[Example]) -> Vec<String> {
    let mut found = Vec::new();
    let mut leaves = Vec::new();
    collect_leaves(tree, "", &mut leaves);

    for (path, command) in &leaves {
        let matching: Vec<&Doc> = docs.iter().filter(|doc| doc.path == path).collect();
        match matching.as_slice() {
            [] if is_pending(command) => {}
            [] => found.push(format!(
                "`jev {path}` has no help doc. Add `Doc {{ path: \"{path}\", .. }}` to DOCS in \
{DOCS_FILE}, in tree order, with `when` (when to use it and which sibling to use instead), \
`input`, `output`, `exit_codes` and 2 to 4 `examples`, one of them machine-readable. The doc of \
`noul` is a complete model."
            )),
            [doc] => check_doc(tree, path, doc, &mut found),
            _ => found.push(format!(
                "`jev {path}` has {} docs in {DOCS_FILE}; keep one",
                matching.len()
            )),
        }
        if command.get_long_about().is_some()
            || command.get_after_help().is_some()
            || command.get_after_long_help().is_some()
        {
            found.push(format!(
                "`jev {path}` sets its own long help in cli.rs (a doc comment of more than one \
paragraph, `long_about`, `after_help` or `after_long_help`). Keep its doc comment to the one-line \
purpose and move the rest into its Doc in {DOCS_FILE}, so that `--help` and `jev spec` agree."
            ));
        }
    }

    for doc in docs {
        if !leaves.iter().any(|(path, _)| *path == doc.path) {
            found.push(format!(
                "{DOCS_FILE} has a doc for `jev {}`, which is not a command: fix its path or \
remove it",
                doc.path
            ));
        }
    }
    for (index, example) in root_examples.iter().enumerate() {
        for problem in check_example(tree, None, example) {
            found.push(format!("`jev` example {}: {problem}", index + 1));
        }
    }
    found
}

/// The visible commands that run something, with their paths.
fn collect_leaves<'a>(
    command: &'a clap::Command,
    path: &str,
    leaves: &mut Vec<(String, &'a clap::Command)>,
) {
    for sub in command.get_subcommands().filter(|sub| !sub.is_hide_set()) {
        let below = join(path, sub.get_name());
        if sub.has_subcommands() {
            collect_leaves(sub, &below, leaves);
        } else {
            leaves.push((below, sub));
        }
    }
}

fn check_doc(tree: &clap::Command, path: &str, doc: &Doc, found: &mut Vec<String>) {
    let mut problem = |text: String| found.push(format!("`jev {path}`: {text} (in {DOCS_FILE})"));

    for (part, text) in [
        ("when", doc.when),
        ("input", doc.input),
        ("output", doc.output),
    ] {
        if text.trim().is_empty() {
            problem(format!("`{part}` is empty"));
        }
    }
    if !names_a_sibling(path, doc.when) {
        problem(
            "`when` names no other command: say when to use a sibling instead, written as \
`jev <command>` in backticks"
                .to_owned(),
        );
    }

    let codes: Vec<u8> = doc.exit_codes.iter().map(|code| code.exit.code()).collect();
    if codes.first() != Some(&0) {
        problem("`exit_codes` must start with 0".to_owned());
    }
    if !codes.windows(2).all(|pair| pair[0] < pair[1]) {
        problem("`exit_codes` must list each code once, in ascending order".to_owned());
    }
    if codes.contains(&1) {
        problem("`exit_codes` leaves out 1: any command can return it".to_owned());
    }
    for exit_code in doc.exit_codes {
        if exit_code.meaning().chars().count() > MEANING_WIDTH {
            problem(format!(
                "the meaning of exit code {} is longer than {MEANING_WIDTH} characters and would \
wrap in --help; shorten it",
                exit_code.exit.code()
            ));
        }
    }

    let count = doc.examples.len();
    if !(2..=4).contains(&count) {
        problem(format!("has {count} examples; give 2 to 4"));
    }
    if !RAW_OUTPUT.contains(&path)
        && !doc.examples.iter().any(|example| {
            MACHINE_READABLE
                .iter()
                .any(|marker| example.command.contains(marker))
        })
    {
        problem(format!(
            "no example is machine-readable: show one using any of {}",
            MACHINE_READABLE
                .map(|marker| format!("`{}`", marker.trim()))
                .join(", ")
        ));
    }
    for (index, example) in doc.examples.iter().enumerate() {
        for text in check_example(tree, Some(path), example) {
            problem(format!("example {}: {text}", index + 1));
        }
    }
}

/// Whether `when` mentions a `jev` command other than the one at `path`, in backticks.
fn names_a_sibling(path: &str, when: &str) -> bool {
    let own = format!("jev {path}");
    when.split('`')
        .skip(1)
        .step_by(2)
        .filter(|quoted| quoted.starts_with("jev "))
        .any(|quoted| quoted != own && !quoted.starts_with(&format!("{own} ")))
}

/// Problems with one example: it must say what it does, run the command it documents, and parse.
fn check_example(tree: &clap::Command, path: Option<&str>, example: &Example) -> Vec<String> {
    let mut found = Vec::new();
    if example.description.trim().is_empty() {
        found.push("`description` is empty".to_owned());
    }
    if let Some(line) = example
        .command
        .lines()
        .find(|line| line.chars().count() > EXAMPLE_WIDTH)
    {
        found.push(format!(
            "a line is longer than {EXAMPLE_WIDTH} characters and would wrap in --help; continue \
it on the next line with `\\`: {line}"
        ));
    }
    let invocations = invocations(example.command);
    if let Some(path) = path {
        let words: Vec<&str> = path.split(' ').collect();
        let runs_it = invocations.iter().any(|invocation| {
            let names: Vec<&str> = invocation
                .iter()
                .skip(1)
                .filter(|word| !word.starts_with('-'))
                .take(words.len())
                .map(String::as_str)
                .collect();
            names == words
        });
        if !runs_it {
            found.push(format!("does not run `jev {path}`"));
        }
    } else if invocations.is_empty() {
        found.push("does not run `jev`".to_owned());
    }
    for invocation in invocations {
        if let Err(error) = tree.clone().try_get_matches_from(&invocation) {
            let rendered = error.render().to_string();
            let reason = rendered.lines().next().unwrap_or_default();
            found.push(format!(
                "`{}` does not parse: {reason}",
                invocation.join(" ")
            ));
        }
    }
    found
}

/// Every `jev` command line in a shell snippet, split into words as the shell would. Enough of
/// the shell for examples: quotes, `\` line continuations, pipes, `;`, `&&`, `if`, `!` and `then`.
/// A command line may also start after `--` in another program's arguments, as in
/// `claude mcp add jev -- jev mcp serve`.
fn invocations(snippet: &str) -> Vec<Vec<String>> {
    let mut commands: Vec<Vec<String>> = vec![Vec::new()];
    let mut word: Option<String> = None;
    let mut quote: Option<char> = None;
    let mut chars = snippet.chars();

    let end_word = |word: &mut Option<String>, commands: &mut Vec<Vec<String>>| {
        if let (Some(done), Some(command)) = (word.take(), commands.last_mut()) {
            command.push(done);
        }
    };
    while let Some(character) = chars.next() {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (Some('"'), '\\') => {
                if let Some(next) = chars.next() {
                    word.get_or_insert_default().push(next);
                }
            }
            (None, '\'' | '"') => {
                quote = Some(character);
                word.get_or_insert_default();
            }
            (None, '\\') => match chars.next() {
                Some('\n') | None => {}
                Some(next) => word.get_or_insert_default().push(next),
            },
            (None, ' ' | '\t') => end_word(&mut word, &mut commands),
            (None, '\n' | '|' | ';' | '&' | '(' | ')' | '<' | '>') => {
                end_word(&mut word, &mut commands);
                commands.push(Vec::new());
            }
            // Inside quotes, or an ordinary character outside them.
            _ => word.get_or_insert_default().push(character),
        }
    }
    end_word(&mut word, &mut commands);

    commands
        .into_iter()
        .map(|command| {
            let start = command
                .windows(2)
                .position(|pair| pair[0] == "--" && pair[1] == "jev")
                .map_or(0, |position| position + 1);
            command
                .into_iter()
                .skip(start)
                .skip_while(|word| ["if", "then", "!", "while", "do"].contains(&word.as_str()))
                .collect::<Vec<_>>()
        })
        .filter(|command| command.first().is_some_and(|first| first == "jev"))
        .collect()
}

#[cfg(test)]
mod tests {
    use clap::{Arg, Args, Command, CommandFactory};

    use super::{invocations, problems};
    use crate::cli::Cli;
    use crate::exit::Exit;
    use crate::help::{DOCS, Doc, Example, ExitCode, ROOT_EXAMPLES, code};

    /// The standard itself: this is the test that fails when a command is added without help.
    #[test]
    fn every_command_meets_the_help_standard() {
        let found = problems(&Cli::command(), DOCS, ROOT_EXAMPLES);

        assert!(
            found.is_empty(),
            "the help standard (CLAUDE.md product rule 1) is not met:\n\n- {}\n",
            found.join("\n- ")
        );
    }

    fn tree() -> Command {
        Command::new("jev").subcommand(
            Command::new("ask")
                .about("Ask")
                .arg(Arg::new("state").long("state")),
        )
    }

    const EXAMPLES: &[Example] = &[
        Example {
            description: "Ask",
            command: "jev ask --state x",
        },
        Example {
            description: "Ask, for a script",
            command: "jev ask --state x -o json",
        },
    ];

    const CODES: &[ExitCode] = &[code(Exit::Success), code(Exit::Usage)];

    fn doc(examples: &'static [Example]) -> Doc {
        Doc {
            path: "ask",
            when: "Use `jev ask` for this; use `jev other` for that.",
            input: "A state.",
            output: "An answer.",
            exit_codes: CODES,
            examples,
        }
    }

    #[test]
    fn a_complete_doc_passes() {
        let tree = tree().arg(Arg::new("output").short('o').global(true));

        assert_eq!(problems(&tree, &[doc(EXAMPLES)], &[]), Vec::<String>::new());
    }

    #[test]
    fn a_command_without_a_doc_fails_and_is_told_what_to_add() {
        let found = problems(&tree(), &[], &[]);

        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("`jev ask` has no help doc"), "{found:?}");
        assert!(found[0].contains("Doc { path: \"ask\", .. }"), "{found:?}");
    }

    #[test]
    fn removing_the_examples_fails() {
        let found = problems(&tree(), &[doc(&[])], &[]);

        assert!(
            found
                .iter()
                .any(|problem| problem.contains("has 0 examples")),
            "{found:?}"
        );
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("machine-readable")),
            "{found:?}"
        );
    }

    #[test]
    fn a_when_section_that_names_no_sibling_fails() {
        let mut lonely = doc(EXAMPLES);
        lonely.when = "Use `jev ask` to ask.";

        let found = problems(&tree(), &[lonely], &[]);

        assert!(
            found
                .iter()
                .any(|problem| problem.contains("names no other command")),
            "{found:?}"
        );
    }

    #[test]
    fn an_example_with_a_flag_that_does_not_exist_fails() {
        const WRONG: &[Example] = &[
            Example {
                description: "Ask",
                command: "jev ask --stat x",
            },
            Example {
                description: "Ask, for a script",
                command: "jev ask --field x",
            },
        ];

        let found = problems(&tree(), &[doc(WRONG)], &[]);

        assert!(
            found
                .iter()
                .any(|problem| problem.contains("example 1: `jev ask --stat x` does not parse")),
            "{found:?}"
        );
    }

    #[test]
    fn a_placeholder_needs_no_doc_but_a_doc_for_nothing_fails() {
        let tree = Command::new("jev").subcommand(crate::cli::Pending::augment_args(
            Command::new("later").about("Later"),
        ));
        let found = problems(&tree, &[doc(EXAMPLES)], &[]);

        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("which is not a command"), "{found:?}");
    }

    #[test]
    fn long_help_written_in_cli_rs_fails() {
        let tree = Command::new("jev").subcommand(
            Command::new("ask")
                .about("Ask")
                .after_help("Examples: ...")
                .arg(Arg::new("state").long("state"))
                .arg(Arg::new("output").short('o')),
        );

        let found = problems(&tree, &[doc(EXAMPLES)], &[]);

        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("sets its own long help"), "{found:?}");
    }

    #[test]
    fn examples_are_split_the_way_a_shell_splits_them() {
        let snippet = "if git log | jev noul \"Is it?\" --state 'a b' \\\n    --fail-under 0.7; then\n  echo \"x\"\nfi\n! jev score x=\"y z\" && jev spec | jq .\nclaude mcp add jev -- jev mcp serve";

        assert_eq!(
            invocations(snippet),
            vec![
                vec![
                    "jev",
                    "noul",
                    "Is it?",
                    "--state",
                    "a b",
                    "--fail-under",
                    "0.7"
                ],
                vec!["jev", "score", "x=y z"],
                vec!["jev", "spec"],
                vec!["jev", "mcp", "serve"],
            ]
        );
    }
}
