//! The worker pool: at most `concurrency` requests in flight, one record per row as it finishes.

use std::collections::BTreeMap;
use std::future::Future;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use jev_client::Transport;
use jev_client::validate::Document;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use super::record::Record;
use super::summary::{StopReason, Summary};
use super::{Ids, Keyed, Mapping, Row, RowProblem, Rows};
use crate::config::Settings;
use crate::error::CliError;
use crate::evaluate::{self, Evaluation, PrepareOptions, Prepared};

/// With `--ordered`, how many rows per worker may be sent or waiting for an earlier row, at most.
/// It bounds the records held back behind a slow row, and so the memory they take.
const ORDERED_WINDOW_PER_WORKER: u64 = 4;

/// Everything a run needs besides its rows, its transport and where its records go.
// Four independent switches of a run, each of them a yes or a no.
#[allow(clippy::struct_excessive_bools)]
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
    /// Read only this many rows of the input.
    pub(crate) limit: Option<u64>,
    /// Write the records in the order of the input rather than as rows finish.
    pub(crate) ordered: bool,
    /// The ids an earlier run already answered (`--resume`): their rows are not sent again.
    pub(crate) done: Option<&'a Ids>,
    /// Once the run is interrupted, how long the rows in flight have to finish. Those that do not
    /// are abandoned without a record, so that a resumed run sends them again.
    pub(crate) grace: Duration,
}

/// How a run ended.
#[derive(Debug)]
pub(crate) struct Outcome {
    pub(crate) summary: Summary,
    /// A row of piped input that could not be used, which stopped the run.
    pub(crate) invalid_input: Option<CliError>,
}

/// Something a caller may want to show while a run goes on. Nothing in the engine prints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Event {
    /// A row was settled: answered, failed, or found already answered.
    Progress(Progress),
    /// The run was interrupted: nothing more is sent, and the rows in flight have `grace` to end.
    Interrupted { in_flight: usize, grace: Duration },
}

/// How far a run has got.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Progress {
    /// Rows in the input, when a preflight has counted them.
    pub(crate) rows_total: Option<u64>,
    pub(crate) ok: u64,
    pub(crate) failed: u64,
    /// Rows not sent because an earlier run answered them.
    pub(crate) already_ok: u64,
    pub(crate) elapsed: Duration,
}

impl Progress {
    /// Rows settled so far, whichever way.
    pub(crate) const fn settled(&self) -> u64 {
        self.ok + self.failed + self.already_ok
    }
}

/// Runs a batch: every row of `rows` becomes one request, and one record written to `sink`.
///
/// The rows are read on a thread of their own, so that a slow producer on a pipe never stalls the
/// requests in flight. Each record is written as one line and flushed at once, in completion order
/// or, with [`Job::ordered`], in input order.
///
/// When `interrupt` completes, nothing more is sent; the rows in flight have [`Job::grace`] to
/// finish and be recorded, and the run returns with [`StopReason::Interrupted`]. A caller without
/// signals passes [`std::future::pending`]. `observe` hears of every settled row, for progress.
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
    interrupt: impl Future<Output = ()>,
    observe: &mut dyn FnMut(Event),
) -> Result<Outcome, CliError> {
    let started = Instant::now();
    let concurrency = job.concurrency.max(1);
    let mut row_receiver = read_rows(rows, job.limit, concurrency);
    let mut ids = job.check_ids.then(Ids::default);
    let mut summary = Summary::default();
    let mut records = Records::new(sink, job.ordered);
    let mut read = 0_u64;
    let mut input_done = false;
    let mut invalid_input = None;
    let mut tasks = JoinSet::new();
    let mut grace_ends: Option<tokio::time::Instant> = None;
    let window =
        ORDERED_WINDOW_PER_WORKER.saturating_mul(u64::try_from(concurrency).unwrap_or(u64::MAX));
    let progress = |summary: &Summary| {
        Event::Progress(Progress {
            rows_total: job.rows_total,
            ok: summary.ok,
            failed: summary.failed,
            already_ok: summary.already_ok,
            elapsed: started.elapsed(),
        })
    };
    tokio::pin!(interrupt);

    loop {
        let can_take = summary.stopped_by.is_none()
            && !input_done
            && tasks.len() < concurrency
            && (!job.ordered || records.unwritten() < window);
        if !can_take && tasks.is_empty() {
            break;
        }
        let grace_over = async {
            match grace_ends {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            biased;
            () = &mut interrupt, if grace_ends.is_none() => {
                summary.stopped_by = Some(StopReason::Interrupted);
                grace_ends = Some(tokio::time::Instant::now() + job.grace);
                observe(Event::Interrupted { in_flight: tasks.len(), grace: job.grace });
            }
            () = grace_over => {
                // What has not finished is abandoned without a record; `--resume` sends it again.
                tasks.shutdown().await;
                break;
            }
            Some(joined) = tasks.join_next(), if !tasks.is_empty() => {
                let (seq, id, result) = joined.unwrap_or_else(|error| {
                    (u64::MAX, Value::Null, Err(CliError::internal(format!("a batch worker failed: {error}"))))
                });
                record(job, &mut summary, &mut records, seq, &id, result)?;
                observe(progress(&summary));
            }
            received = row_receiver.recv(), if can_take => {
                let Some(row) = received else {
                    input_done = true;
                    continue;
                };
                read += 1;
                match admit(job, &mut ids, row) {
                    Admitted::Send(id, prepared) => {
                        let seq = records.next_seq();
                        let transport = Arc::clone(&transport);
                        let price = job.usd_per_mtok;
                        tasks.spawn(async move {
                            let result = evaluate::send(&transport, &prepared, price).await;
                            (seq, id, result)
                        });
                    }
                    // Nothing was sent for it: the row is recorded as failed, and cost nothing.
                    Admitted::Refused(id, error) => {
                        let seq = records.next_seq();
                        record(job, &mut summary, &mut records, seq, &id, Err(error))?;
                        observe(progress(&summary));
                    }
                    Admitted::AlreadyOk => {
                        summary.already_ok += 1;
                        observe(progress(&summary));
                    }
                    Admitted::Unusable(error) => {
                        invalid_input = Some(error);
                        summary.stopped_by = Some(StopReason::InvalidInput);
                    }
                }
            }
            else => break,
        }
    }
    // Stops the reader, if it is still going.
    drop(row_receiver);
    // An interrupted `--ordered` run still writes every record it has, after a gap.
    records.flush_all(&mut summary)?;

    summary.rows_total = job.rows_total.unwrap_or(read).max(read);
    summary.finish(started.elapsed());
    Ok(Outcome {
        summary,
        invalid_input,
    })
}

/// Where the records go: straight out, or held back until the rows before them are written.
struct Records<'a> {
    sink: &'a mut dyn Write,
    ordered: bool,
    /// The number the next row sent will have.
    next_seq: u64,
    /// The number of the next record to write, in input order.
    next_to_write: u64,
    /// Finished records waiting for an earlier row.
    waiting: BTreeMap<u64, Value>,
}

impl<'a> Records<'a> {
    fn new(sink: &'a mut dyn Write, ordered: bool) -> Self {
        Self {
            sink,
            ordered,
            next_seq: 0,
            next_to_write: 0,
            waiting: BTreeMap::new(),
        }
    }

    /// Numbers a row about to be settled.
    fn next_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }

    /// Rows numbered whose records have not been written yet: in flight or waiting.
    fn unwritten(&self) -> u64 {
        self.next_seq - self.next_to_write
    }

    fn put(&mut self, seq: u64, record: Value, summary: &mut Summary) -> Result<(), CliError> {
        if !self.ordered {
            return write_record(self.sink, &record, summary);
        }
        self.waiting.insert(seq, record);
        while let Some(record) = self.waiting.remove(&self.next_to_write) {
            write_record(self.sink, &record, summary)?;
            self.next_to_write += 1;
        }
        Ok(())
    }

    /// Writes whatever is still waiting, in order, past the gaps left by abandoned rows.
    fn flush_all(&mut self, summary: &mut Summary) -> Result<(), CliError> {
        for record in std::mem::take(&mut self.waiting).into_values() {
            write_record(self.sink, &record, summary)?;
        }
        Ok(())
    }
}

/// Reads the rows on a thread of their own, handing them over through a channel that holds at most
/// `capacity`.
fn read_rows(
    rows: Rows,
    limit: Option<u64>,
    capacity: usize,
) -> mpsc::Receiver<Result<Row, RowProblem>> {
    let limit = limit.map_or(usize::MAX, |limit| {
        usize::try_from(limit).unwrap_or(usize::MAX)
    });
    let (sender, receiver) = mpsc::channel(capacity);
    // Detached: if it is blocked on a pipe when the run stops, the process ending takes it along.
    std::thread::spawn(move || {
        for row in rows.take(limit) {
            if sender.blocking_send(row).is_err() {
                break;
            }
        }
    });
    receiver
}

/// What becomes of a row that was read.
enum Admitted {
    /// Send it.
    Send(Value, Prepared),
    /// Its request fails validation: record the failure without sending anything.
    Refused(Value, CliError),
    /// An earlier run answered it.
    AlreadyOk,
    /// It cannot be used at all, and sending stops there.
    Unusable(CliError),
}

fn admit(job: &Job<'_>, ids: &mut Option<Ids>, row: Result<Row, RowProblem>) -> Admitted {
    let keyed = row.and_then(|row| {
        let line = row.line;
        let keyed = job.mapping.key(row)?;
        if let Some(ids) = ids {
            ids.insert(&keyed.id, line)?;
        }
        Ok(keyed)
    });
    let Keyed { id, state } = match keyed {
        Ok(keyed) => keyed,
        Err(problem) => {
            return Admitted::Unusable(
                CliError::usage(format!(
                    "the input cannot be used at {}; sending stopped there",
                    problem.describe()
                ))
                .hint("fix the input; with --input <file> every row is checked before anything is sent"),
            );
        }
    };
    if job.done.is_some_and(|done| done.contains(&id)) {
        return Admitted::AlreadyOk;
    }
    let options = PrepareOptions {
        state: Some(state),
        strict: job.strict,
        skip_size_check: job.skip_size_check,
    };
    match evaluate::prepare(job.questions.clone(), job.settings, &options) {
        Ok(prepared) => Admitted::Send(id, prepared),
        Err(error) => Admitted::Refused(id, error),
    }
}

/// Counts a settled row, writes its record, and stops the run if too many rows have failed.
fn record(
    job: &Job<'_>,
    summary: &mut Summary,
    records: &mut Records<'_>,
    seq: u64,
    id: &Value,
    result: Result<Evaluation, CliError>,
) -> Result<(), CliError> {
    let record = settle(summary, id, result);
    records.put(seq, record, summary)?;
    stop_after_failure(job, summary);
    Ok(())
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
            encode(&Record::ok(id, &envelope))
        }
        Err(error) => {
            summary.failed += 1;
            summary.retries += u64::from(error.attempts.saturating_sub(1));
            encode(&Record::failed(id, &error))
        }
    }
}

fn encode(record: &Record<'_>) -> Value {
    serde_json::to_value(record).unwrap_or(Value::Null)
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
    use std::fmt::Write as _;
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

    use super::{Event, Ids, Job, run};
    use crate::batch::summary::StopReason;
    use crate::batch::{Mapping, RowFormat, Rows, StateMapping};
    use crate::config::{Flags, Settings};
    use crate::env::Env;

    /// A transport that sleeps a little, counts how many calls are in flight, fails any state
    /// containing `FAIL`, takes a minute over one containing `HANG`, and takes `n` ms over a
    /// state `{"ms": n}`.
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
            let state = request.state.as_value();
            let fail = state.to_string().contains("FAIL");
            let pause = if state.to_string().contains("HANG") {
                Duration::from_secs(60)
            } else {
                Duration::from_millis(state.get("ms").and_then(Value::as_u64).unwrap_or(2))
            };
            async move {
                let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                self.most.fetch_max(now, Ordering::SeqCst);
                self.calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(pause).await;
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
            limit: None,
            ordered: false,
            done: None,
            grace: Duration::from_secs(10),
        };
        let mut sink = Vec::new();

        let outcome = block_on(run(
            &job,
            rows(input),
            Arc::clone(&transport),
            &mut sink,
            std::future::pending(),
            &mut |_| {},
        ))
        .unwrap();

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
            limit: None,
            ordered: false,
            done: None,
            grace: Duration::from_secs(10),
        };
        let mut sink = Vec::new();

        let outcome = block_on(run(
            &job,
            rows(input),
            Arc::clone(&transport),
            &mut sink,
            std::future::pending(),
            &mut |_| {},
        ))
        .unwrap();

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
            limit: None,
            ordered: false,
            done: None,
            grace: Duration::from_secs(10),
        };
        let mut sink = Vec::new();

        let outcome = block_on(run(
            &job,
            rows(input),
            Arc::clone(&transport),
            &mut sink,
            std::future::pending(),
            &mut |_| {},
        ))
        .unwrap();

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
            limit: None,
            ordered: false,
            done: None,
            grace: Duration::from_secs(10),
        };
        let mut sink = Vec::new();

        let outcome = block_on(run(
            &job,
            rows(input),
            Arc::clone(&transport),
            &mut sink,
            std::future::pending(),
            &mut |_| {},
        ))
        .unwrap();

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

    fn job<'a>(questions: &'a Document, settings: &'a Settings, mapping: &'a Mapping) -> Job<'a> {
        Job {
            questions,
            settings,
            mapping,
            concurrency: 4,
            max_errors: None,
            strict: false,
            skip_size_check: false,
            usd_per_mtok: None,
            rows_total: None,
            check_ids: false,
            limit: None,
            ordered: false,
            done: None,
            grace: Duration::from_secs(10),
        }
    }

    fn ids(sink: &[u8]) -> Vec<u64> {
        records(sink)
            .iter()
            .map(|record| record["id"].as_u64().unwrap())
            .collect()
    }

    #[test]
    fn ordered_records_follow_the_input_even_when_later_rows_finish_first() {
        // Row n takes 40 - 2n ms: the last rows finish first.
        let input = (0..20).fold(String::new(), |mut input, n| {
            let _ = writeln!(input, "{{\"ms\": {}}}", 40 - 2 * n);
            input
        });
        let (questions, settings) = (questions(), settings());
        let mapping = Mapping {
            state: StateMapping::WholeRow,
            id_field: None,
        };
        let transport = Arc::new(Counting::default());
        let unordered = job(&questions, &settings, &mapping);
        let ordered = Job {
            ordered: true,
            ..job(&questions, &settings, &mapping)
        };
        let (mut first, mut second) = (Vec::new(), Vec::new());

        block_on(run(
            &unordered,
            rows(input.clone()),
            Arc::clone(&transport),
            &mut first,
            std::future::pending(),
            &mut |_| {},
        ))
        .unwrap();
        block_on(run(
            &ordered,
            rows(input),
            Arc::clone(&transport),
            &mut second,
            std::future::pending(),
            &mut |_| {},
        ))
        .unwrap();

        assert_ne!(
            ids(&first),
            (1..=20).collect::<Vec<_>>(),
            "completion order differs"
        );
        assert_eq!(ids(&second), (1..=20).collect::<Vec<_>>());
        assert_eq!(transport.most.load(Ordering::SeqCst), 4, "still concurrent");
    }

    #[test]
    fn resumed_ids_are_not_sent_again_and_a_limit_stops_reading() {
        let input =
            "{\"id\": \"a\"}\n{\"id\": 2}\n{\"id\": \"c\"}\n{\"id\": \"d\"}\n{\"id\": \"e\"}\n"
                .to_owned();
        let (questions, settings) = (questions(), settings());
        let mapping = Mapping {
            state: StateMapping::WholeRow,
            id_field: Some("id".into()),
        };
        let mut done = Ids::default();
        done.add(&json!("a"));
        done.add(&json!("2"));
        let transport = Arc::new(Counting::default());
        let job = Job {
            done: Some(&done),
            limit: Some(4),
            ..job(&questions, &settings, &mapping)
        };
        let mut sink = Vec::new();
        let mut settled = Vec::new();

        let outcome = block_on(run(
            &job,
            rows(input),
            Arc::clone(&transport),
            &mut sink,
            std::future::pending(),
            &mut |event| {
                if let Event::Progress(progress) = event {
                    settled.push(progress.settled());
                }
            },
        ))
        .unwrap();

        let mut sent: Vec<String> = records(&sink)
            .iter()
            .map(|record| record["id"].as_str().unwrap().to_owned())
            .collect();
        sent.sort();
        assert_eq!(sent, ["c", "d"]);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
        let summary = outcome.summary;
        assert_eq!(
            (
                summary.rows_total,
                summary.ok,
                summary.already_ok,
                summary.skipped
            ),
            (4, 2, 2, 0)
        );
        assert_eq!(settled, [1, 2, 3, 4]);
    }

    #[test]
    fn an_interrupt_stops_sending_and_abandons_what_outlasts_the_grace() {
        let mut input = "\"HANG\"\n".to_owned();
        input.push_str(&"\"fine\"\n".repeat(500));
        let (questions, settings) = (questions(), settings());
        let mapping = Mapping {
            state: StateMapping::WholeRow,
            id_field: None,
        };
        let transport = Arc::new(Counting::default());
        let job = Job {
            concurrency: 2,
            rows_total: Some(501),
            grace: Duration::from_millis(50),
            ..job(&questions, &settings, &mapping)
        };
        let mut sink = Vec::new();
        let mut events = Vec::new();

        let outcome = block_on(run(
            &job,
            rows(input),
            Arc::clone(&transport),
            &mut sink,
            async { tokio::time::sleep(Duration::from_millis(100)).await },
            &mut |event| {
                if let Event::Interrupted { .. } = event {
                    events.push(event);
                }
            },
        ))
        .unwrap();

        let records = records(&sink);
        assert!(
            !records.is_empty() && records.len() < 500,
            "{}",
            records.len()
        );
        assert!(
            records
                .iter()
                .all(|record| record["status"] == "ok" && record["id"] != 1)
        );
        assert_eq!(
            events,
            [Event::Interrupted {
                in_flight: 2,
                grace: Duration::from_millis(50)
            }]
        );
        let summary = outcome.summary;
        assert_eq!(summary.stopped_by, Some(StopReason::Interrupted));
        assert_eq!(summary.ok, records.len() as u64);
        assert_eq!(
            summary.skipped,
            501 - summary.ok,
            "the abandoned row counts as skipped"
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
            limit: None,
            ordered: false,
            done: None,
            grace: Duration::from_secs(10),
        };

        let outcome = block_on(run(
            &job,
            rows(input),
            Arc::clone(&transport),
            &mut Closed,
            std::future::pending(),
            &mut |_| {},
        ))
        .unwrap();

        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        assert_eq!(outcome.summary.stopped_by, Some(StopReason::OutputClosed));
        assert_eq!(outcome.summary.skipped, 19);
    }
}
