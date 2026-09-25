//! `jev validate`: every offline check, with nothing sent and nothing billed.

use std::fmt::Write as _;

use indexmap::IndexMap;
use jev_client::validate::{self, Finding, Options, Report, Severity, SizeEstimate};
use serde::Serialize;
use serde_json::Value;

use super::Context;
use crate::cli::ValidateArgs;
use crate::error::CliError;
use crate::input::{self, StateSource};
use crate::output::{Render, Ui, printable};

pub(crate) fn run(arguments: &ValidateArgs, context: &mut Context<'_>) -> Result<(), CliError> {
    let mut document =
        input::read_document(&arguments.file, arguments.input_format, context.stdin)?;

    // A state is only ever read when it was asked for. Unlike `jev eval`, a pipe is not read
    // implicitly: validating a questions-only file is normal, and must not wait on stdin.
    let state_source = match (&arguments.state, arguments.state_file.as_deref()) {
        (Some(text), _) => Some(StateSource::inline(text)?),
        (None, Some("-")) if arguments.file == "-" => {
            return Err(CliError::usage("stdin is already being read as the request file (-f -), so it cannot also supply the state")
                .hint("pass the state with --state or --state-file <path>"));
        }
        (None, Some("-")) => Some(StateSource::Stdin),
        (None, Some(path)) => Some(StateSource::File(path.to_owned())),
        (None, None) => None,
    };
    if let Some(source) = state_source {
        let state = input::read_state(&source, arguments.state_format, context.stdin)?;
        if let Value::Object(root) = document.value_mut() {
            root.insert("state".to_owned(), state);
        }
    }

    // A request file may leave the model and the state to whoever evaluates it.
    let options = Options::default()
        .strict(arguments.strict)
        .skip_size_check(arguments.skip_size_check)
        .model_optional(true)
        .state_optional(true);
    let report = validate::check_document(&document, &options);
    let outcome = Validation::new(&arguments.file, report, arguments.strict);
    context.output.emit(&outcome, context.stdout)?;

    if outcome.valid {
        return Ok(());
    }
    let errors = outcome.summary.errors;
    Err(CliError::usage(format!(
        "{} is not valid: {errors} problem{} found",
        outcome.file,
        if errors == 1 { "" } else { "s" }
    ))
    .hint(if arguments.strict {
        "fix the findings above; --strict counts warnings as errors"
    } else {
        "fix the findings above"
    }))
}

/// The result of `jev validate`. This shape is versioned public API.
#[derive(Debug, Serialize)]
pub(crate) struct Validation {
    /// The file that was checked, or `stdin`.
    file: String,
    /// `true` when nothing stands in the way of evaluating the request. Warnings do not count,
    /// unless `--strict` was given.
    valid: bool,
    /// Whether warnings were counted as errors.
    strict: bool,
    summary: Summary,
    /// Every finding, in document order, then lints, then size.
    findings: Vec<Finding>,
    /// The estimated input tokens, when the request has a state and the check was not skipped.
    size: Option<SizeEstimate>,
}

#[derive(Debug, Serialize)]
struct Summary {
    errors: usize,
    warnings: usize,
}

impl Validation {
    pub(crate) fn new(file: &str, report: Report, strict: bool) -> Self {
        let summary = Summary {
            errors: report.errors().count(),
            warnings: report.warnings().count(),
        };
        Self {
            file: if file == "-" {
                "stdin".to_owned()
            } else {
                file.to_owned()
            },
            valid: report.is_valid(),
            strict,
            summary,
            findings: report.findings,
            size: report.size,
        }
    }
}

impl Render for Validation {
    fn human(&self, ui: Ui) -> String {
        let verdict = if self.valid {
            ui.bold("valid")
        } else {
            ui.error_label("invalid")
        };
        // A finding quotes the document it checked, and the file name came from the command
        // line, so both are neutralised before a terminal sees them.
        let mut text = format!(
            "{}: {verdict}{}\n",
            printable(&self.file),
            counts(&self.summary)
        );

        // Grouped by question, in the order the questions first appear.
        let mut groups: IndexMap<Option<&str>, Vec<&Finding>> = IndexMap::new();
        for finding in &self.findings {
            groups
                .entry(finding.question.as_deref())
                .or_default()
                .push(finding);
        }
        for (question, findings) in groups {
            let heading =
                question.map_or_else(|| "request".to_owned(), |id| format!("question `{id}`"));
            let _ = write!(text, "\n{}\n", ui.bold(&heading));
            for finding in findings {
                let label = match finding.severity {
                    Severity::Error => ui.error_label("error  "),
                    _ => ui.warning_label("warning"),
                };
                let place = if finding.path.is_empty() {
                    "/"
                } else {
                    finding.path.as_str()
                };
                let _ = writeln!(text, "  {label}  {}  {}", finding.rule.id(), ui.dim(place));
                let _ = writeln!(text, "           {}", printable(&finding.message));
                if let Some(suggestion) = &finding.suggestion {
                    let _ = writeln!(
                        text,
                        "           {} {}",
                        ui.dim("fix:"),
                        printable(suggestion)
                    );
                }
            }
        }
        if let Some(size) = &self.size {
            let _ = write!(
                text,
                "\n{}\n",
                ui.dim(&format!(
                    "estimated input tokens: {} of {} in total, {} of {} for the state plus the longest question",
                    size.total_tokens, size.total_budget_tokens, size.largest_question_tokens, size.question_budget_tokens
                ))
            );
        }
        text
    }

    fn records(&self) -> Option<Vec<serde_json::Value>> {
        Some(
            self.findings
                .iter()
                .filter_map(|finding| serde_json::to_value(finding).ok())
                .collect(),
        )
    }
}

fn counts(summary: &Summary) -> String {
    let plural =
        |count: usize, word: &str| format!("{count} {word}{}", if count == 1 { "" } else { "s" });
    match (summary.errors, summary.warnings) {
        (0, 0) => String::new(),
        (errors, 0) => format!(", {}", plural(errors, "error")),
        (0, warnings) => format!(", {}", plural(warnings, "warning")),
        (errors, warnings) => format!(
            ", {}, {}",
            plural(errors, "error"),
            plural(warnings, "warning")
        ),
    }
}
