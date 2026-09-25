//! The `batch_run` tool: `jev batch run` over files inside the `--allow-dir` directories.
//!
//! It drives the same batch engine as the command, so it writes the same records. Before anything
//! is sent, every row is checked and priced as `jev batch run --dry-run` does, and the run is
//! refused if it is over `--max-batch-rows` or `--max-batch-cost-usd`.

use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jev_client::Transport;
use jev_client::validate::Document;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::roots::Roots;
use super::{Reporter, Session, with_session};
use crate::batch::{self, Event, Job, Mapping, Plan, RowFormat, Rows, StateMapping, Summary};
use crate::config::{Key, Value as SettingValue};
use crate::error::CliError;
use crate::evaluate::{self, PrepareOptions};
use crate::input;

/// How often a progress notification is sent, at most. The last row always gets one.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// What a server started with `--allow-dir` has for `batch_run`.
pub(super) struct Access<T> {
    pub(super) roots: Roots,
    /// A transport of its own, with one throttle for every row of every run: a 429 or 529 on any
    /// row slows them all.
    pub(super) transport: Result<Arc<T>, CliError>,
    pub(super) max_rows: Option<u64>,
    pub(super) max_cost_usd: Option<f64>,
}

/// The arguments of `batch_run`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Arguments {
    /// The questions, keyed by ids you choose, as in `evaluate`. Give this or `questions_file`.
    questions: Option<Map<String, Value>>,
    /// A question-set file (JSON or YAML, as `jev batch run -f` reads) inside an allowed
    /// directory. Any `state` in it is ignored. Give this or `questions`.
    questions_file: Option<String>,
    /// The model; it replaces a `model` in `questions_file`.
    model: Option<String>,
    /// The rows: a JSONL or CSV file, or a JSON array, inside an allowed directory, one request
    /// per row.
    input: String,
    /// The format of `input`: `jsonl`, `csv`, or `json` for one array of rows (at most 50 MB).
    /// Default: `csv` for a `.csv` file, `json` for a `.json` file holding one array, `jsonl`
    /// otherwise.
    input_format: Option<RowFormat>,
    /// Where the records go: a file inside an allowed directory that does not exist yet. One JSON
    /// line per row, as `jev batch run --out` writes.
    out: String,
    /// Send this one field of each row as its state. Default: the whole row.
    state_field: Option<String>,
    /// Send an object of only these fields of each row as its state.
    state_fields: Option<Vec<String>>,
    /// The field that identifies each row in the records; its values must be unique. Default:
    /// the line number, or the element's index in a JSON array.
    id_field: Option<String>,
    /// Read only the first N rows of the input.
    #[schemars(range(min = 1))]
    limit: Option<u64>,
    /// Write the records in input order instead of as rows finish.
    #[serde(default)]
    ordered: bool,
    /// Stop sending once N rows have failed; rows in flight still finish.
    #[schemars(range(min = 1))]
    max_errors: Option<u64>,
    /// Treat validation warnings as errors.
    #[serde(default)]
    strict: bool,
}

/// What `batch_run` returns: the summary `jev batch run` reports, and where the records are.
#[derive(Serialize)]
struct Ran<'a> {
    #[serde(flatten)]
    summary: &'a Summary,
    /// The file the records were written to, resolved.
    out: String,
}

pub(super) fn call<T: Transport + 'static>(
    session: &mut Session<T>,
    arguments: &Map<String, Value>,
    reporter: &mut Reporter<'_>,
) -> Result<Value, CliError> {
    let Some(access) = &session.batch else {
        return Err(CliError::internal(
            "`batch_run` was called on a server started without --allow-dir",
        ));
    };
    let arguments: Arguments =
        serde_json::from_value(Value::Object(arguments.clone())).map_err(|error| {
            CliError::usage(format!(
                "the arguments of `batch_run` are not valid: {error}"
            ))
            .hint("pass the arguments the tool's input schema describes")
        })?;
    let mapping = arguments.mapping()?;
    let questions = arguments.question_set(&access.roots)?;
    let input = access.roots.existing_file(&arguments.input, "input")?;
    let format = arguments.input_format.unwrap_or_else(|| {
        RowFormat::of_path(Path::new(&arguments.input), || {
            access.roots.open(&input).ok()
        })
    });
    let out = access.roots.new_file(&arguments.out, "out")?;

    // The question set is checked once, with a stand-in state, before any row is read.
    let checked = evaluate::prepare(
        questions.clone(),
        &session.settings,
        &PrepareOptions {
            state: Some(json!("a stand-in for each row's state")),
            strict: arguments.strict,
            skip_size_check: true,
        },
    )?;
    let concurrency = match session.settings.get(Key::Concurrency).value {
        Some(SettingValue::Count(count)) => count,
        _ => 4,
    };
    let open_rows = || -> Result<Rows, CliError> {
        let rows = Rows::new(Box::new(access.roots.open(&input)?), format)?;
        if let Some(columns) = rows.columns() {
            mapping.check_columns(columns).map_err(|error| {
                error.hint("check `state_field`, `state_fields` and `id_field` against the columns")
            })?;
        }
        Ok(rows)
    };
    let mut job = Job {
        questions: &questions,
        settings: &session.settings,
        mapping: &mapping,
        concurrency: usize::try_from(concurrency).unwrap_or(1),
        max_errors: arguments.max_errors,
        strict: arguments.strict,
        skip_size_check: false,
        usd_per_mtok: session.price_override,
        rows_total: None,
        check_ids: false,
        limit: arguments.limit,
        ordered: arguments.ordered,
        done: None,
        grace: Duration::ZERO,
    };

    // Every row is checked and priced first, so that a bad row or a cap costs nothing.
    let limit = usize::try_from(arguments.limit.unwrap_or(u64::MAX)).unwrap_or(usize::MAX);
    let plan = Plan::new(open_rows()?.take(limit), &job, &checked.requested_model);
    if plan.invalid_rows > 0 {
        return Err(batch::invalid_input(&plan.problems, plan.invalid_rows).hint(
            "fix the input, or the `state_field`, `state_fields` or `id_field` that name its fields",
        ));
    }
    access.check_caps(&plan)?;
    let transport = Arc::clone(access.transport.as_ref().map_err(Clone::clone)?);
    job.rows_total = Some(plan.rows_total);

    let rows = open_rows()?;
    let mut file = access.roots.create(&out)?;
    let mut meter = Meter {
        reporter,
        last: None,
    };
    let mut observe = |event: Event| meter.observe(event);
    let outcome = session.runtime.block_on(batch::run(
        &job,
        rows,
        transport,
        &mut file,
        std::future::pending(),
        &mut observe,
    ))?;
    if let Some(error) = outcome.invalid_input {
        return Err(error);
    }

    let summary = outcome.summary;
    session.spend.record_batch(summary.ok, summary.cost_usd);
    let warnings: Vec<_> = checked.report.warnings().cloned().collect();
    with_session(
        &Ran {
            summary: &summary,
            out: out.display().to_string(),
        },
        &warnings,
        session.spend(),
    )
}

/// Progress, as `notifications/progress`: at most one every [`PROGRESS_EVERY`], and always one
/// for the last row.
struct Meter<'r, 'w> {
    reporter: &'r mut Reporter<'w>,
    last: Option<Instant>,
}

impl Meter<'_, '_> {
    fn observe(&mut self, event: Event) {
        let Event::Progress(progress) = event else {
            return;
        };
        let settled = progress.settled();
        let finished = progress.rows_total == Some(settled);
        if self
            .last
            .is_some_and(|last| last.elapsed() < PROGRESS_EVERY)
            && !finished
        {
            return;
        }
        self.last = Some(Instant::now());
        let rows = progress.rows_total.map_or_else(
            || format!("{settled} rows"),
            |total| format!("{settled}/{total} rows"),
        );
        self.reporter.progress(
            settled,
            progress.rows_total,
            &format!("{rows}: {} ok, {} failed", progress.ok, progress.failed),
        );
    }
}

impl<T> Access<T> {
    /// Refuses a run over `--max-batch-rows`, or over `--max-batch-cost-usd` or of a cost that
    /// cannot be known.
    fn check_caps(&self, plan: &Plan) -> Result<(), CliError> {
        if let Some(limit) = self.max_rows
            && plan.rows_total > limit
        {
            let mut error = CliError::usage(format!(
                "this run has {} rows, above the limit of {limit} rows per run; nothing was sent",
                plan.rows_total
            ))
            .hint("pass `limit` to run the first rows only, split the input, or restart the server with a higher --max-batch-rows");
            error.code = "row_limit";
            error.details = Some(json!({ "rows": plan.rows_total, "max_batch_rows": limit }));
            return Err(error);
        }
        let Some(limit) = self.max_cost_usd else {
            return Ok(());
        };
        let model = &plan.requested_model;
        // An alias has no price, and a guardrail that cannot be checked refuses.
        let estimate = plan.estimated_cost_usd.ok_or_else(|| {
            CliError::cost_limit(format!(
                "`{model}` has no known price, so this run cannot be checked against --max-batch-cost-usd; nothing was sent"
            ))
            .hint("use a versioned model id with a known price, such as `jev-1.13.0`: pass `model`, or start the server with --model; or set `usd_per_mtok` under `[pricing]` in config.toml")
        })?;
        if estimate <= limit {
            return Ok(());
        }
        let tokens = plan.estimated_input_tokens.unwrap_or_default();
        let mut error = CliError::cost_limit(format!(
            "this run is estimated at ${estimate:.6} ({} rows, {tokens} input tokens), above the limit of ${limit:.6} per run; nothing was sent",
            plan.rows_total
        ))
        .hint("pass `limit` to run the first rows only, trim the state with `state_field` or `state_fields`, or restart the server with a higher --max-batch-cost-usd");
        error.details = Some(json!({
            "estimated_cost_usd": estimate,
            "estimated_input_tokens": tokens,
            "rows": plan.rows_total,
            "max_batch_cost_usd": limit
        }));
        Err(error)
    }
}

impl Arguments {
    /// The question set, from `questions` or `questions_file`, with `model` when it is given.
    fn question_set(&self, roots: &Roots) -> Result<Document, CliError> {
        let mut document = match (&self.questions, &self.questions_file) {
            (Some(questions), None) => {
                Document::from_value(json!({ "questions": Value::Object(questions.clone()) }))
            }
            (None, Some(path)) => {
                let resolved = roots.existing_file(path, "questions_file")?;
                let mut text = String::new();
                roots
                    .open(&resolved)?
                    .read_to_string(&mut text)
                    .map_err(|error| {
                        CliError::usage(format!("cannot read `questions_file` {path}: {error}"))
                            .hint("pass a readable question-set file")
                    })?;
                // Any `state` in it is replaced by each row's, as `jev batch run` does.
                input::parse_document(&text, path, path, None)?
            }
            _ => {
                return Err(CliError::usage(
                    "`batch_run` needs the questions: give either `questions` or `questions_file`",
                )
                .hint("pass the questions inline as `questions`, or the path of a question-set file as `questions_file`"));
            }
        };
        if let (Some(model), Value::Object(document)) = (&self.model, document.value_mut()) {
            document.insert("model".to_owned(), json!(model));
        }
        Ok(document)
    }

    /// How each row becomes an id and a state.
    fn mapping(&self) -> Result<Mapping, CliError> {
        let empty = |argument: &str| {
            CliError::usage(format!("`{argument}` names an empty field")).hint(
                "name the fields as they appear in the input, e.g. `state_fields`: [\"subject\", \"body\"]",
            )
        };
        let state = match (&self.state_field, &self.state_fields) {
            (Some(_), Some(_)) => {
                return Err(CliError::usage(
                    "give `state_field` or `state_fields`, not both",
                )
                .hint("`state_field` sends one field as the state, `state_fields` an object of several"));
            }
            (Some(field), None) if field.trim().is_empty() => return Err(empty("state_field")),
            (Some(field), None) => StateMapping::Field(field.clone()),
            (None, Some(fields)) if fields.is_empty() => return Err(empty("state_fields")),
            (None, Some(fields)) => {
                let mut trimmed: Vec<String> = Vec::new();
                for field in fields {
                    let field = field.trim();
                    if field.is_empty() {
                        return Err(empty("state_fields"));
                    }
                    if trimmed.iter().any(|known| known == field) {
                        return Err(CliError::usage(format!(
                            "`state_fields` names `{field}` more than once"
                        ))
                        .hint("list every field once"));
                    }
                    trimmed.push(field.to_owned());
                }
                StateMapping::Fields(trimmed)
            }
            (None, None) => StateMapping::WholeRow,
        };
        for (argument, value) in [("limit", self.limit), ("max_errors", self.max_errors)] {
            if value == Some(0) {
                return Err(CliError::usage(format!("`{argument}` must be at least 1"))
                    .hint(format!("leave `{argument}` out for no limit")));
            }
        }
        if self
            .id_field
            .as_ref()
            .is_some_and(|field| field.trim().is_empty())
        {
            return Err(empty("id_field"));
        }
        Ok(Mapping {
            state,
            id_field: self.id_field.clone(),
        })
    }
}
