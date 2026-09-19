//! The worker pool: at most `concurrency` requests in flight, one record per row as it finishes.

use std::io::{self, Write};
use std::sync::Arc;
use std::time::Instant;

use jev_client::Transport;
use jev_client::validate::Document;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use super::summary::{StopReason, Summary};
use super::{Ids, Keyed, Mapping, Row, RowProblem, Rows};
use crate::config::Settings;
use crate::error::CliError;
use crate::evaluate::{self, Evaluation, PrepareOptions};

/// Everything a run needs besides its rows, its transport and where its records go.
#[derive(Debug)]
pub(crate) struct Job<'a> {
    /// The question set, and optionally the model. Each row's state replaces any state in it.
    pub(crate) questions: &'a Document,
    pub(crate) settings: &'a Settings,
    pub(crate) mapping: &'a Mapping,
    /// Requests in flight at once; at least 1.
    pub(crate) concurrency: usize,
    /// Stop sending once this many rows have failed.
    pub(crate) max_errors: Option<u64>,
    pub(crate) strict: bool,
    pub(crate) skip_size_check: bool,
    /// A configured price per million input tokens, for the cost estimate.
    pub(crate) usd_per_mtok: Option<f64>,
    /// Rows in the input, when a preflight has counted them. A run that stops early reports the
    /// rows it never reached as skipped.
    pub(crate) rows_total: Option<u64>,
    /// Whether ids still need checking for repeats, which a preflight has done already.
    pub(crate) check_ids: bool,
}

/// How a run ended.
#[derive(Debug)]
pub(crate) struct Outcome {
    pub(crate) summary: Summary,
    /// A row of piped input that could not be used, which stopped the run.
    pub(crate) invalid_input: Option<CliError>,
}

/// Runs a batch: every row of `rows` becomes one request, and one record written to `sink`.
///
/// The rows are read on a thread of their own, so that a slow producer on a pipe never stalls the
/// requests in flight. Records are written in completion order, one line each, flushed at once.
///
/// # Errors
///
/// An internal error when a record cannot be written. Everything that goes wrong with a row is
/// recorded instead, and the run goes on.
pub(crate) async fn run<T: Transport + 'static>(
    job: &Job<'_>,
    rows: Rows,
    transport: Arc<T>,
    sink: &mut dyn Write,
) -> Result<Outcome, CliError> {
    let started = Instant::now();
    let concurrency = job.concurrency.max(1);
    let (row_sender, mut row_receiver) = mpsc::channel::<Result<Row, RowProblem>>(concurrency);
    // Detached: if it is blocked on a pipe when the run stops, the process ending takes it along.
    std::thread::spawn(move || {
        for row in rows {
            if row_sender.blocking_send(row).is_err() {
                break;
            }
        }
    });

    let mut ids = job.check_ids.then(Ids::default);
    let mut summary = Summary::default();
    let mut read = 0_u64;
    let mut input_done = false;
    let mut invalid_input = None;
    let mut tasks = JoinSet::new();

    loop {
        let can_take = summary.stopped_by.is_none() && !input_done && tasks.len() < concurrency;
        if !can_take && tasks.is_empty() {
            break;
        }
        tokio::select! {
            biased;
            Some(joined) = tasks.join_next(), if !tasks.is_empty() => {
                let (id, result) = joined.unwrap_or_else(|error| {
                    (Value::Null, Err(CliError::internal(format!("a batch worker failed: {error}"))))
                });
                let record = settle(&mut summary, &id, result);
                write_record(sink, &record, &mut summary)?;
                stop_after_failure(job, &mut summary);
            }
            received = row_receiver.recv(), if can_take => {
                let Some(row) = received else {
                    input_done = true;
                    continue;
                };
                read += 1;
                let keyed = row.and_then(|row| {
                    let line = row.line;
                    let keyed = job.mapping.key(row)?;
                    if let Some(ids) = &mut ids {
                        ids.insert(&keyed.id, line)?;
                    }
                    Ok(keyed)
                });
                let Keyed { id, state } = match keyed {
                    Ok(keyed) => keyed,
                    Err(problem) => {
                        invalid_input = Some(
                            CliError::usage(format!(
                                "the input cannot be used at {}; sending stopped there",
                                problem.describe()
                            ))
                            .hint("fix the input; with --input <file> every row is checked before anything is sent"),
                        );
                        summary.stopped_by = Some(StopReason::InvalidInput);
                        continue;
                    }
                };
                let options = PrepareOptions {
                    state: Some(state),
                    strict: job.strict,
                    skip_size_check: job.skip_size_check,
                };
                match evaluate::prepare(job.questions.clone(), job.settings, &options) {
                    // Nothing was sent for it: the row is recorded as failed, and cost nothing.
                    Err(error) => {
                        let record = settle(&mut summary, &id, Err(error));
                        write_record(sink, &record, &mut summary)?;
                        stop_after_failure(job, &mut summary);
                    }
                    Ok(prepared) => {
                        let transport = Arc::clone(&transport);
                        let price = job.usd_per_mtok;
                        tasks.spawn(async move {
                            let result = evaluate::send(&transport, &prepared, price).await;
                            (id, result)
                        });
                    }
                }
            }
            else => break,
        }
    }
    // Stops the reader, if it is still going.
    drop(row_receiver);

    summary.rows_total = job.rows_total.unwrap_or(read).max(read);
    summary.finish(started.elapsed());
    Ok(Outcome {
        summary,
        invalid_input,
    })
}

/// Counts a finished row and builds its record.
fn settle(summary: &mut Summary, id: &Value, result: Result<Evaluation, CliError>) -> Value {
    match result {
        Ok(evaluation) => {
            let envelope = evaluation.envelope;
            summary.ok += 1;
            summary.retries += u64::from(evaluation.attempts.saturating_sub(1));
            summary.input_tokens += envelope.usage.input_tokens;
            // Unknown for one row is unknown for the total: an estimate is never a guess.
            summary.cost_usd = match (summary.ok, summary.cost_usd, envelope.cost_usd) {
                (1, _, cost) => cost,
                (_, Some(total), Some(cost)) => Some(total + cost),
                _ => None,
            };
            if !summary.models.contains(&envelope.model) {
                summary.models.push(envelope.model.clone());
            }
            json!({
                "id": id,
                "status": "ok",
                "model": envelope.model,
                "answers": envelope.answers,
                "usage": envelope.usage,
                "cost_usd": envelope.cost_usd,
                "request_id": envelope.request_id,
                "latency_ms": envelope.latency_ms,
            })
        }
        Err(error) => {
            summary.failed += 1;
            summary.retries += u64::from(error.attempts.saturating_sub(1));
            let mut body = error.to_json();
            let error = body.get_mut("error").map(Value::take);
            json!({ "id": id, "status": "error", "error": error })
        }
    }
}

fn stop_after_failure(job: &Job<'_>, summary: &mut Summary) {
    if summary.stopped_by.is_none() && job.max_errors.is_some_and(|limit| summary.failed >= limit) {
        summary.stopped_by = Some(if job.max_errors == Some(1) {
            StopReason::FailFast
        } else {
            StopReason::MaxErrors
        });
    }
}

/// Writes one record as one line, and flushes it, so that the output is valid JSONL at every
/// moment.
///
/// A reader that has gone away (`| head`, say) has what it wanted: sending stops, and no more
/// records are written, but that is not an error.
fn write_record(
    sink: &mut dyn Write,
    record: &Value,
    summary: &mut Summary,
) -> Result<(), CliError> {
    if summary.stopped_by == Some(StopReason::OutputClosed) {
        return Ok(());
    }
    let mut line = record.to_string();
    line.push('\n');
    match sink.write_all(line.as_bytes()).and_then(|()| sink.flush()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {
            summary.stopped_by = Some(StopReason::OutputClosed);
            Ok(())
        }
        Err(error) => Err(CliError::internal(format!(
            "could not write a result record: {error}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use indexmap::IndexMap;
    use jev_client::validate::Document;
    use jev_client::{
        Answer, Error, ErrorKind, ModelList, NoulAnswer, Reply, ReplyMeta, Request, Response,
        Transport, Usage,
    };
    use serde_json::{Value, json};

    use super::{Job, run};
    use crate::batch::summary::StopReason;
    use crate::batch::{Mapping, RowFormat, Rows, StateMapping};
    use crate::config::{Flags, Settings};
    use crate::env::Env;

    /// A transport that sleeps a little, counts how many calls are in flight, and fails any
    /// state containing `FAIL`.
    #[derive(Default)]
    struct Counting {
        in_flight: AtomicUsize,
        most: AtomicUsize,
        calls: AtomicUsize,
    }

    impl Transport for Counting {
        fn evaluate(
            &self,
            request: &Request,
        ) -> impl Future<Output = Result<Reply<Response>, Error>> + Send {
            let fail = request.state.as_value().to_string().contains("FAIL");
            async move {
                let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                self.most.fetch_max(now, Ordering::SeqCst);
                self.calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(2)).await;
                self.in_flight.fetch_sub(1, Ordering::SeqCst);
                if fail {
                    return Err(Error::new(ErrorKind::Unprocessable, "Bad state.")
                        .with_status(422)
                        .with_error_type("invalid_request_error")
                        .with_request_id(Some("req_bad".to_owned()))
                        .with_attempts(1));
                }
                let answers =
                    IndexMap::from([("q".to_owned(), Answer::from(NoulAnswer::new(0.9)))]);
                Ok(Reply::new(
                    Response::new("jev-1.13.0", answers, Usage::new(100, 0)),
                    ReplyMeta::new(Some("req_ok".into()), Duration::from_millis(2), 2),
                ))
            }
        }

        fn list_models(&self) -> impl Future<Output = Result<Reply<ModelList>, Error>> + Send {
            std::future::ready(Ok(Reply::new(ModelList::default(), ReplyMeta::default())))
        }
    }

    fn questions() -> Document {
        Document::from_value(
            json!({ "questions": { "q": { "type": "noul", "instructions": "Is it urgent?" } } }),
        )
    }

    fn settings() -> Settings {
        Settings::resolve(&Flags::default(), &Env::default(), None, &IndexMap::new()).unwrap()
    }

    fn rows(text: String) -> Rows {
        Rows::new(Box::new(std::io::Cursor::new(text)), RowFormat::Jsonl).unwrap()
    }

    fn records(sink: &[u8]) -> Vec<Value> {
        String::from_utf8(sink.to_vec())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn block_on<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn requests_in_flight_never_exceed_the_concurrency_and_every_row_gets_a_record() {
        let input = "{\"n\": 0}\n".repeat(60);
        let (questions, settings) = (questions(), settings());
        let mapping = Mapping {
            state: StateMapping::WholeRow,
            id_field: None,
        };
        let transport = Arc::new(Counting::default());
        let job = Job {
            questions: &questions,
            settings: &settings,
            mapping: &mapping,
            concurrency: 3,
            max_errors: None,
            strict: false,
            skip_size_check: false,
            usd_per_mtok: None,
            rows_total: None,
            check_ids: true,
        };
        let mut sink = Vec::new();

        let outcome = block_on(run(&job, rows(input), Arc::clone(&transport), &mut sink)).unwrap();

        assert_eq!(transport.most.load(Ordering::SeqCst), 3);
        let records = records(&sink);
        assert_eq!(records.len(), 60);
        let mut ids: Vec<u64> = records
            .iter()
            .map(|record| record["id"].as_u64().unwrap())
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, (1..=60).collect::<Vec<_>>());
        assert_eq!(records[0]["status"], "ok");
        assert_eq!(records[0]["request_id"], "req_ok");
        let summary = outcome.summary;
        assert_eq!(
            (
                summary.rows_total,
                summary.ok,
                summary.failed,
                summary.skipped
            ),
            (60, 60, 0, 0)
        );
        assert_eq!(summary.input_tokens, 6000);
        assert_eq!(summary.retries, 60, "each answer took two attempts");
        assert_eq!(summary.models, ["jev-1.13.0"]);
        assert!(outcome.invalid_input.is_none());
    }

    #[test]
    fn a_failing_row_is_recorded_and_fail_fast_stops_sending() {
        let input =
            "{\"s\": \"ok\"}\n{\"s\": \"FAIL\"}\n{\"s\": \"ok\"}\n{\"s\": \"ok\"}\n".to_owned();
        let (questions, settings) = (questions(), settings());
        let mapping = Mapping {
            state: StateMapping::Field("s".into()),
            id_field: None,
        };
        let transport = Arc::new(Counting::default());
        let job = Job {
            questions: &questions,
            settings: &settings,
            mapping: &mapping,
            concurrency: 1,
            max_errors: Some(1),
            strict: false,
            skip_size_check: false,
            usd_per_mtok: None,
            rows_total: Some(4),
            check_ids: false,
        };
        let mut sink = Vec::new();

        let outcome = block_on(run(&job, rows(input), Arc::clone(&transport), &mut sink)).unwrap();

        let records = records(&sink);
        assert_eq!(records.len(), 2);
        assert_eq!(records[1]["id"], 2);
        assert_eq!(records[1]["status"], "error");
        assert_eq!(records[1]["error"]["error_type"], "invalid_request_error");
        assert_eq!(records[1]["error"]["request_id"], "req_bad");
        assert_eq!(records[1]["error"]["exit_code"], 4);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
        let summary = outcome.summary;
        assert_eq!((summary.ok, summary.failed, summary.skipped), (1, 1, 2));
        assert_eq!(summary.stopped_by, Some(StopReason::FailFast));
    }

    #[test]
    fn a_row_that_fails_validation_is_recorded_without_being_sent() {
        // Far over the size budget, so that validation refuses it.
        let input = format!(
            "{{\"s\": \"{}\"}}\n{{\"s\": \"fine\"}}\n",
            "word ".repeat(100_000)
        );
        let (questions, settings) = (questions(), settings());
        let mapping = Mapping {
            state: StateMapping::Field("s".into()),
            id_field: None,
        };
        let transport = Arc::new(Counting::default());
        let job = Job {
            questions: &questions,
            settings: &settings,
            mapping: &mapping,
            concurrency: 2,
            max_errors: None,
            strict: false,
            skip_size_check: false,
            usd_per_mtok: None,
            rows_total: None,
            check_ids: false,
        };
        let mut sink = Vec::new();

        let outcome = block_on(run(&job, rows(input), Arc::clone(&transport), &mut sink)).unwrap();

        let records = records(&sink);
        let failed = records.iter().find(|record| record["id"] == 1).unwrap();
        assert_eq!(failed["status"], "error");
        assert_eq!(failed["error"]["code"], "usage");
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        assert_eq!((outcome.summary.ok, outcome.summary.failed), (1, 1));
    }

    #[test]
    fn a_repeated_id_in_piped_rows_stops_sending_there() {
        let input =
            "{\"id\": \"a\"}\n{\"id\": \"b\"}\n{\"id\": \"a\"}\n{\"id\": \"c\"}\n".to_owned();
        let (questions, settings) = (questions(), settings());
        let mapping = Mapping {
            state: StateMapping::WholeRow,
            id_field: Some("id".into()),
        };
        let transport = Arc::new(Counting::default());
        let job = Job {
            questions: &questions,
            settings: &settings,
            mapping: &mapping,
            concurrency: 1,
            max_errors: None,
            strict: false,
            skip_size_check: false,
            usd_per_mtok: None,
            rows_total: None,
            check_ids: true,
        };
        let mut sink = Vec::new();

        let outcome = block_on(run(&job, rows(input), Arc::clone(&transport), &mut sink)).unwrap();

        assert_eq!(records(&sink).len(), 2);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
        assert_eq!(outcome.summary.stopped_by, Some(StopReason::InvalidInput));
        assert_eq!(outcome.summary.skipped, 1);
        let error = outcome.invalid_input.unwrap();
        assert_eq!(error.exit.code(), 2);
        assert!(
            error.message.starts_with(
                "the input cannot be used at line 3: the id `a` is used by an earlier row"
            ),
            "{}",
            error.message
        );
    }

    /// A reader that has gone away, like `head` after its lines.
    struct Closed;

    impl std::io::Write for Closed {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_closed_output_stops_sending_without_an_error() {
        let input = "{\"n\": 0}\n".repeat(20);
        let (questions, settings) = (questions(), settings());
        let mapping = Mapping {
            state: StateMapping::WholeRow,
            id_field: None,
        };
        let transport = Arc::new(Counting::default());
        let job = Job {
            questions: &questions,
            settings: &settings,
            mapping: &mapping,
            concurrency: 1,
            max_errors: None,
            strict: false,
            skip_size_check: false,
            usd_per_mtok: None,
            rows_total: Some(20),
            check_ids: false,
        };

        let outcome =
            block_on(run(&job, rows(input), Arc::clone(&transport), &mut Closed)).unwrap();

        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        assert_eq!(outcome.summary.stopped_by, Some(StopReason::OutputClosed));
        assert_eq!(outcome.summary.skipped, 19);
    }
}
