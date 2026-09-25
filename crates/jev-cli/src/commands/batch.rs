//! `jev batch run`: one question set over every row of a JSONL, CSV or JSON array input.
//!
//! Unlike every other command it streams its result: one record per row is written as each row
//! finishes, so that a long run can be followed and a crash loses nothing already answered. The
//! summary is not the result, so it goes to stderr, and to `--summary-json` when asked for.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jev_client::validate::Document;
use jev_client::{HttpTransport, Throttle};
use serde_json::{Value, json};

use super::Context;
use super::eval::block_on;
use crate::batch::{
    self, Event, Job, Mapping, Plan, Progress, RowFormat, RowSource, StateMapping, StopReason,
    Summary,
};
use crate::cli::BatchRunArgs;
use crate::config::{Key, Value as SettingValue};
use crate::error::CliError;
use crate::evaluate::{self, PrepareOptions, Prepared};
use crate::exit::Exit;
use crate::input;
use crate::interrupt;
use crate::notice::{Notice, Notifier};
use crate::output::Ui;

/// Above this many requests in flight, shared keys are commonly rate limited.
const ADVISED_CONCURRENCY: u32 = 8;

/// Once interrupted, how long the requests in flight have to finish and be recorded.
const INTERRUPT_GRACE: Duration = Duration::from_secs(10);

/// How often a progress bar is redrawn, at most.
const BAR_EVERY: Duration = Duration::from_millis(100);

/// How often a line of progress is written when stderr is not a terminal.
const LINE_EVERY: Duration = Duration::from_secs(5);

pub(crate) fn run(arguments: &BatchRunArgs, context: &mut Context<'_>) -> Result<Exit, CliError> {
    if context.output.field.is_some() && !arguments.dry_run {
        return Err(CliError::usage(
            "--field does not apply to `jev batch run`, which writes one JSON record per row",
        )
        .hint("select fields from the records afterwards, e.g. with `jq`; --field works with --dry-run"));
    }
    let source = row_source(arguments, context.interaction.stdin_is_terminal())?;
    let format = row_format(arguments, &source)?;
    let mapping = mapping(arguments)?;
    let resumed = match &arguments.out {
        Some(path) if arguments.resume => Some(batch::read_resumed(path)?),
        Some(path) => {
            refuse_existing_output(path)?;
            None
        }
        None => None,
    };

    let (questions, checked, concurrency) = check_questions(arguments, context)?;
    let settings = context.settings()?.clone();
    let limit = |rows: batch::Rows| {
        rows.take(arguments.limit.map_or(usize::MAX, |limit| {
            usize::try_from(limit).unwrap_or(usize::MAX)
        }))
    };
    let mut job = Job {
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
        rows_total: None,
        check_ids: false,
        limit: arguments.limit,
        ordered: arguments.ordered,
        done: resumed.as_ref().map(|resumed| &resumed.done),
        grace: INTERRUPT_GRACE,
    };

    if arguments.dry_run {
        let plan = Plan::new(
            limit(open_rows(&source, format, &mapping)?),
            &job,
            &checked.requested_model,
        );
        context.output.emit(&plan, context.stdout)?;
        return if plan.invalid_rows == 0 {
            Ok(Exit::Success)
        } else {
            Err(batch::invalid_input(&plan.problems, plan.invalid_rows))
        };
    }

    // A file is read in full first, so that a bad row or a repeated id costs nothing.
    if source.is_rereadable() {
        let rows = open_rows(&source, format, &mapping)?;
        job.rows_total = Some(batch::preflight(limit(rows), &mapping)?);
    }
    job.check_ids = job.rows_total.is_none() && mapping.id_field.is_some();
    let rows = open_rows(&source, format, &mapping)?;

    // One throttle for every worker: a 429 or 529 on any of them slows them all.
    let transport = Arc::new(
        context
            .transport(&settings)?
            .with_throttle(Arc::new(Throttle::new())),
    );
    if let (Some(path), Some(resumed)) = (&arguments.out, &resumed)
        && resumed.partial_len > 0
    {
        batch::discard_partial(path, resumed)?;
        context.notify(&Notice::warning(
            "partial_record_discarded",
            format!("the last line of {path} was cut short, probably by a crash; it was removed, and its row will be sent again"),
        ));
    }
    let outcome = send(&job, rows, transport, arguments, context)?;
    conclude(&outcome, arguments, context, &checked.requested_model)
}

/// Sends every row, with progress on stderr and SIGINT or SIGTERM stopping the run cleanly.
fn send(
    job: &Job<'_>,
    rows: batch::Rows,
    transport: Arc<HttpTransport>,
    arguments: &BatchRunArgs,
    context: &mut Context<'_>,
) -> Result<batch::Outcome, CliError> {
    let notifier = context.notifier;
    let mut meter = Meter::new(notifier);
    let outcome = {
        let mut observe = |event: Event| meter.observe(event);
        let interrupt = || {
            interrupt::first_of_two(move || {
                notifier.emit(&Notice::warning(
                    "interrupted",
                    "interrupted again: stopping now, without waiting for the rows in flight",
                ));
            })
        };
        match &arguments.out {
            Some(path) => {
                let mut file = open_output(path, arguments.resume)?;
                block_on(async {
                    batch::run(job, rows, transport, &mut file, interrupt(), &mut observe).await
                })?
            }
            None => block_on(async {
                batch::run(
                    job,
                    rows,
                    transport,
                    &mut *context.stdout,
                    interrupt(),
                    &mut observe,
                )
                .await
            })?,
        }
    };
    meter.clear();
    Ok(outcome)
}

/// Reports the run, and decides its exit code.
fn conclude(
    outcome: &batch::Outcome,
    arguments: &BatchRunArgs,
    context: &Context<'_>,
    requested_model: &str,
) -> Result<Exit, CliError> {
    report(&outcome.summary, context);
    if let Some(path) = &arguments.summary_json {
        write_summary(path, &outcome.summary)?;
    }
    let remind = arguments.warn_unpinned
        || context.settings()?.get(Key::WarnUnpinned).value == Some(SettingValue::Switch(true));
    if remind {
        for model in &outcome.summary.models {
            if let Some(notice) = evaluate::unpinned_notice(requested_model, model) {
                context.notify(&notice);
            }
        }
    }
    if outcome.summary.stopped_by == Some(StopReason::Interrupted) {
        return Ok(Exit::Interrupted);
    }
    if let Some(error) = &outcome.invalid_input {
        return Err(error.clone());
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
        (RowSource::Stdin, Some(RowFormat::Json)) => Err(CliError::usage(
            "rows on stdin must be JSONL; a JSON array is read from a file",
        )
        .hint("pass the file with --input <file>, or pipe `jq -c '.[]'` of it as JSONL")),
        (_, Some(format)) => Ok(format),
        (RowSource::File(path), None) => Ok(RowFormat::of_path(Path::new(path), || {
            File::open(path).ok()
        })),
        (RowSource::Stdin, None) => Ok(RowFormat::Jsonl),
    }
}

/// Opens the rows, and checks that a CSV header has every column the mapping names.
fn open_rows(
    source: &RowSource,
    format: RowFormat,
    mapping: &Mapping,
) -> Result<batch::Rows, CliError> {
    let rows = source.open(format)?;
    if let Some(columns) = rows.columns() {
        mapping.check_columns(columns)?;
    }
    Ok(rows)
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
        .hint("pass --resume to continue that run, or choose another --out path")),
        _ => Ok(()),
    }
}

fn open_output(path: &str, resume: bool) -> Result<File, CliError> {
    if !resume {
        refuse_existing_output(path)?;
    }
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

/// Progress on stderr: a bar redrawn in place on a terminal, a line every few seconds elsewhere
/// (FR-BATCH-10), nothing under --quiet.
struct Meter {
    notifier: Notifier,
    style: MeterStyle,
    last: Option<Instant>,
    /// Whether a bar is on the screen, to be erased before anything else is written.
    drawn: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MeterStyle {
    Bar,
    Lines,
    Off,
}

impl Meter {
    fn new(notifier: Notifier) -> Self {
        let style = if notifier.quiet {
            MeterStyle::Off
        } else if notifier.terminal {
            MeterStyle::Bar
        } else {
            MeterStyle::Lines
        };
        Self {
            notifier,
            style,
            last: None,
            drawn: false,
        }
    }

    fn observe(&mut self, event: Event) {
        match event {
            Event::Progress(progress) => self.progress(&progress),
            Event::Interrupted { in_flight, grace } => {
                self.clear();
                self.notifier.emit(
                    &Notice::warning(
                        "interrupted",
                        format!(
                            "interrupted: nothing more will be sent; waiting up to {} s for {in_flight} request{} in flight",
                            grace.as_secs(),
                            if in_flight == 1 { "" } else { "s" }
                        ),
                    )
                    .hint("interrupt again to stop at once; rerun with --resume to continue"),
                );
            }
        }
    }

    fn progress(&mut self, progress: &Progress) {
        let every = match self.style {
            MeterStyle::Off => return,
            MeterStyle::Bar => BAR_EVERY,
            MeterStyle::Lines => LINE_EVERY,
        };
        let now = Instant::now();
        match self.last {
            Some(last) if now.duration_since(last) < every => return,
            // The first line waits a whole interval: a short run needs no progress.
            None if self.style == MeterStyle::Lines && progress.elapsed < every => return,
            _ => self.last = Some(now),
        }
        let mut stderr = io::stderr().lock();
        if self.style == MeterStyle::Bar {
            let _ = write!(stderr, "\r\x1b[2K{}", bar_line(progress, self.notifier.ui));
            let _ = stderr.flush();
            self.drawn = true;
        } else if self.notifier.format.is_machine_readable() {
            let _ = writeln!(stderr, "{}", json!({ "progress": progress_json(progress) }));
        } else {
            let _ = writeln!(stderr, "{}", progress_line(progress, self.notifier.ui));
        }
    }

    /// Erases the bar, so that what follows starts on a clean line.
    fn clear(&mut self) {
        if self.drawn {
            let _ = write!(io::stderr().lock(), "\r\x1b[2K");
            self.drawn = false;
        }
    }
}

fn rate(progress: &Progress) -> f64 {
    let seconds = progress.elapsed.as_secs_f64();
    // A row count is far below 2^52.
    #[allow(clippy::cast_precision_loss)]
    let sent = (progress.ok + progress.failed) as f64;
    if seconds > 0.0 { sent / seconds } else { 0.0 }
}

fn bar_line(progress: &Progress, ui: Ui) -> String {
    let settled = progress.settled();
    let mut facts = vec![match progress.rows_total {
        #[allow(clippy::cast_precision_loss)] // Row counts are far below 2^52.
        Some(total) if total > 0 => format!(
            "{} {settled}/{total} rows",
            ui.bar(settled as f64 / total as f64, 20)
        ),
        _ => format!("{settled} rows"),
    }];
    if progress.failed > 0 {
        facts.push(format!("{} failed", progress.failed));
    }
    let rate = rate(progress);
    facts.push(format!("{rate:.1} rows/s"));
    if let (Some(total), true) = (progress.rows_total, rate > 0.0) {
        #[allow(clippy::cast_precision_loss)] // Row counts are far below 2^52.
        let left = total.saturating_sub(settled) as f64 / rate;
        facts.push(format!("ETA {}", eta(left)));
    }
    facts.join(ui.separator())
}

/// A time left, as `42s`, `3m05s` or `2h10m`.
fn eta(seconds: f64) -> String {
    // Rounded, non-negative and capped far below `u64::MAX`, so the cast is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let seconds = seconds.clamp(0.0, 1e7).round() as u64;
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m{:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h{:02}m", seconds / 3600, seconds % 3600 / 60),
    }
}

fn progress_line(progress: &Progress, ui: Ui) -> String {
    let settled = progress.settled();
    let rows = progress.rows_total.map_or_else(
        || format!("{settled} rows"),
        |total| format!("{settled}/{total} rows"),
    );
    format!(
        "{} {rows}: {} ok, {} failed{}, {:.1} rows/s",
        ui.bold("batch:"),
        progress.ok,
        progress.failed,
        if progress.already_ok > 0 {
            format!(", {} already ok", progress.already_ok)
        } else {
            String::new()
        },
        rate(progress)
    )
}

fn progress_json(progress: &Progress) -> Value {
    json!({
        "rows_total": progress.rows_total,
        "ok": progress.ok,
        "failed": progress.failed,
        "already_ok": progress.already_ok,
        "elapsed_ms": u64::try_from(progress.elapsed.as_millis()).unwrap_or(u64::MAX),
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Meter, MeterStyle, bar_line, eta, progress_json, progress_line};
    use crate::batch::Progress;
    use crate::notice::Notifier;
    use crate::output::{Format, Ui};

    fn progress(rows_total: Option<u64>) -> Progress {
        Progress {
            rows_total,
            ok: 38,
            failed: 2,
            already_ok: 10,
            elapsed: Duration::from_secs(4),
        }
    }

    #[test]
    fn a_bar_shows_the_share_done_the_rate_and_the_time_left() {
        assert_eq!(
            bar_line(&progress(Some(100)), Ui::plain()),
            "##########.......... 50/100 rows | 2 failed | 10.0 rows/s | ETA 5s"
        );
        assert_eq!(
            bar_line(&progress(None), Ui::plain()),
            "50 rows | 2 failed | 10.0 rows/s"
        );
        assert_eq!(
            [eta(0.4), eta(185.0), eta(7900.0), eta(f64::INFINITY)],
            ["0s", "3m05s", "2h11m", "2777h46m"]
        );
    }

    #[test]
    fn a_progress_line_is_text_for_a_person_and_json_for_a_program() {
        assert_eq!(
            progress_line(&progress(Some(100)), Ui::plain()),
            "batch: 50/100 rows: 38 ok, 2 failed, 10 already ok, 10.0 rows/s"
        );
        assert_eq!(
            progress_json(&progress(None)).to_string(),
            r#"{"rows_total":null,"ok":38,"failed":2,"already_ok":10,"elapsed_ms":4000}"#
        );
    }

    #[test]
    fn a_terminal_gets_a_bar_a_pipe_gets_lines_and_quiet_gets_nothing() {
        let notifier = |quiet, terminal| Notifier {
            format: Format::Json,
            ui: Ui::plain(),
            quiet,
            terminal,
        };

        let styles = [(false, true), (false, false), (true, true), (true, false)]
            .map(|(quiet, terminal)| Meter::new(notifier(quiet, terminal)).style);

        assert_eq!(
            styles,
            [
                MeterStyle::Bar,
                MeterStyle::Lines,
                MeterStyle::Off,
                MeterStyle::Off
            ]
        );
    }
}
