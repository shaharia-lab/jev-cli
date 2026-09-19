//! `jev batch run`: one question set over every row of a JSONL or CSV input.
//!
//! Unlike every other command it streams its result: one record per row is written as each row
//! finishes, so that a long run can be followed and a crash loses nothing already answered. The
//! summary is not the result, so it goes to stderr, and to `--summary-json` when asked for.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;

use jev_client::validate::Document;
use serde_json::{Value, json};

use super::Context;
use super::eval::block_on;
use crate::batch::{self, Job, Mapping, RowFormat, RowSource, StateMapping, Summary};
use crate::cli::BatchRunArgs;
use crate::config::{Key, Value as SettingValue};
use crate::error::CliError;
use crate::evaluate::{self, PrepareOptions, Prepared};
use crate::exit::Exit;
use crate::input;
use crate::notice::Notice;

/// Above this many requests in flight, shared keys are commonly rate limited.
const ADVISED_CONCURRENCY: u32 = 8;

pub(crate) fn run(arguments: &BatchRunArgs, context: &mut Context<'_>) -> Result<Exit, CliError> {
    if context.output.field.is_some() {
        return Err(CliError::usage(
            "--field does not apply to `jev batch run`, which writes one JSON record per row",
        )
        .hint("select fields from the records afterwards, e.g. with `jq`"));
    }
    let source = row_source(arguments, context.interaction.stdin_is_terminal())?;
    let format = row_format(arguments, &source)?;
    let mapping = mapping(arguments)?;
    if let Some(path) = &arguments.out {
        refuse_existing_output(path)?;
    }

    let (questions, checked, concurrency) = check_questions(arguments, context)?;
    let settings = context.settings()?.clone();

    // A file is read in full first, so that a bad row or a repeated id costs nothing.
    let rows_total = if source.is_rereadable() {
        let rows = source.open(format)?;
        if let Some(columns) = rows.columns() {
            mapping.check_columns(columns)?;
        }
        Some(batch::preflight(rows, &mapping)?)
    } else {
        None
    };
    let rows = source.open(format)?;
    if let Some(columns) = rows.columns() {
        mapping.check_columns(columns)?;
    }

    let transport = Arc::new(context.transport(&settings)?);
    let job = Job {
        questions: &questions,
        settings: &settings,
        mapping: &mapping,
        concurrency: usize::try_from(concurrency).unwrap_or(1),
        max_errors: if arguments.fail_fast {
            Some(1)
        } else {
            arguments.max_errors
        },
        strict: arguments.strict,
        skip_size_check: arguments.skip_size_check,
        usd_per_mtok: context.config_file()?.pricing_usd_per_mtok,
        rows_total,
        check_ids: rows_total.is_none() && mapping.id_field.is_some(),
    };
    let outcome = match &arguments.out {
        Some(path) => {
            let mut file = open_output(path)?;
            block_on(batch::run(&job, rows, transport, &mut file))?
        }
        None => block_on(batch::run(&job, rows, transport, &mut *context.stdout))?,
    };

    report(&outcome.summary, context);
    if let Some(path) = &arguments.summary_json {
        write_summary(path, &outcome.summary)?;
    }
    let remind = arguments.warn_unpinned
        || settings.get(Key::WarnUnpinned).value == Some(SettingValue::Switch(true));
    if remind {
        for model in &outcome.summary.models {
            if let Some(notice) = evaluate::unpinned_notice(&checked.requested_model, model) {
                context.notify(&notice);
            }
        }
    }
    if let Some(error) = outcome.invalid_input {
        return Err(error);
    }
    Ok(if outcome.summary.failed > 0 {
        Exit::BatchPartial
    } else {
        Exit::Success
    })
}

/// Reads and validates the question set once, with a stand-in state, before any row is read, and
/// resolves the concurrency.
fn check_questions(
    arguments: &BatchRunArgs,
    context: &mut Context<'_>,
) -> Result<(Document, Prepared, u32), CliError> {
    let questions = input::read_document(&arguments.file, None, context.stdin)?;
    if questions.value().get("state").is_some() {
        context.notify(&Notice::warning(
            "state_ignored",
            format!(
                "the `state` in {} is ignored: each row supplies its own",
                arguments.file
            ),
        ));
    }
    let settings = context.settings()?;
    let checked = evaluate::prepare(
        questions.clone(),
        settings,
        &PrepareOptions {
            state: Some(json!("a stand-in for each row's state")),
            strict: arguments.strict,
            skip_size_check: true,
        },
    )?;
    for warning in checked.warnings() {
        context.notify(&warning);
    }
    let concurrency = match settings.get(Key::Concurrency).value {
        Some(SettingValue::Count(count)) => count,
        _ => 4,
    };
    if concurrency > ADVISED_CONCURRENCY {
        context.notify(
            &Notice::warning(
                "high_concurrency",
                format!("{concurrency} requests in flight at once; shared API keys are commonly rate limited above {ADVISED_CONCURRENCY}"),
            )
            .hint("if rows fail with `rate_limited`, lower --concurrency"),
        );
    }
    Ok((questions, checked, concurrency))
}

/// Where the rows come from. A pipe is read only when nothing else supplies rows.
fn row_source(arguments: &BatchRunArgs, stdin_is_terminal: bool) -> Result<RowSource, CliError> {
    let source = match arguments.input.as_deref() {
        Some("-") => RowSource::Stdin,
        Some(path) => RowSource::File(path.to_owned()),
        None if stdin_is_terminal => {
            return Err(CliError::usage("there are no rows to evaluate")
                .hint("pass --input <file>, or pipe JSONL rows in"));
        }
        None => RowSource::Stdin,
    };
    if source == RowSource::Stdin && arguments.file == "-" {
        return Err(CliError::usage(
            "stdin cannot supply both the question set (-f -) and the rows",
        )
        .hint("put the question set in a file, or the rows with --input <file>"));
    }
    Ok(source)
}

fn row_format(arguments: &BatchRunArgs, source: &RowSource) -> Result<RowFormat, CliError> {
    match (source, arguments.input_format) {
        (RowSource::Stdin, Some(RowFormat::Csv)) => Err(CliError::usage(
            "rows on stdin must be JSONL; CSV is read from a file",
        )
        .hint("pass the CSV file with --input <file>")),
        (_, Some(format)) => Ok(format),
        (RowSource::File(path), None) => Ok(RowFormat::of_path(Path::new(path))),
        (RowSource::Stdin, None) => Ok(RowFormat::Jsonl),
    }
}

fn mapping(arguments: &BatchRunArgs) -> Result<Mapping, CliError> {
    let empty = |flag: &str| {
        CliError::usage(format!("{flag} names an empty field"))
            .hint("name the fields as they appear in the input, e.g. --state-fields subject,body")
    };
    let state = if let Some(field) = &arguments.state_field {
        if field.trim().is_empty() {
            return Err(empty("--state-field"));
        }
        StateMapping::Field(field.clone())
    } else if arguments.state_fields.is_empty() {
        StateMapping::WholeRow
    } else {
        let mut fields: Vec<String> = Vec::new();
        for field in &arguments.state_fields {
            let field = field.trim();
            if field.is_empty() {
                return Err(empty("--state-fields"));
            }
            if fields.iter().any(|known| known == field) {
                return Err(CliError::usage(format!(
                    "--state-fields names `{field}` more than once"
                ))
                .hint("list every field once"));
            }
            fields.push(field.to_owned());
        }
        StateMapping::Fields(fields)
    };
    if arguments
        .id_field
        .as_ref()
        .is_some_and(|field| field.trim().is_empty())
    {
        return Err(empty("--id-field"));
    }
    Ok(Mapping {
        state,
        id_field: arguments.id_field.clone(),
    })
}

/// Records are appended, never overwritten: an output that already holds some is refused.
fn refuse_existing_output(path: &str) -> Result<(), CliError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.len() > 0 => Err(CliError::usage(format!(
            "the output file {path} already holds records"
        ))
        .hint("choose another --out path, or move that file away first")),
        _ => Ok(()),
    }
}

fn open_output(path: &str) -> Result<File, CliError> {
    refuse_existing_output(path)?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| {
            CliError::usage(format!("cannot write the output file {path}: {error}"))
                .hint("check the path passed to --out")
        })
}

/// The summary on stderr: text for a person, one JSON line for a program, nothing under --quiet.
fn report(summary: &Summary, context: &Context<'_>) {
    let notifier = context.notifier;
    if notifier.quiet {
        return;
    }
    let text = if notifier.format.is_machine_readable() {
        json!({ "summary": summary }).to_string()
    } else {
        summary.human(notifier.ui)
    };
    let _ = writeln!(io::stderr().lock(), "{text}");
}

fn write_summary(path: &str, summary: &Summary) -> Result<(), CliError> {
    let text = serde_json::to_string_pretty(summary).unwrap_or_else(|_| Value::Null.to_string());
    fs::write(path, format!("{text}\n")).map_err(|error| {
        CliError::usage(format!("cannot write the summary to {path}: {error}"))
            .hint("check the path passed to --summary-json; the records were written")
    })
}
