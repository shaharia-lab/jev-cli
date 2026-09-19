//! What a batch run reports when it ends.

use std::fmt::Write as _;
use std::time::Duration;

use serde::Serialize;

use crate::output::Ui;

/// Why a run stopped sending before the input ran out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StopReason {
    /// `--fail-fast`: a row failed.
    FailFast,
    /// `--max-errors`: that many rows failed.
    MaxErrors,
    /// A row of piped input could not be used.
    InvalidInput,
    /// Whoever read the records stopped reading them.
    OutputClosed,
}

/// The end-of-run summary. This shape is versioned public API, like a result record.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(crate) struct Summary {
    /// Rows in the input. For piped input that stopped early, the rows read so far.
    pub(crate) rows_total: u64,
    /// Rows answered.
    pub(crate) ok: u64,
    /// Rows that failed, each with an error record.
    pub(crate) failed: u64,
    /// Rows not attempted because the run stopped early.
    pub(crate) skipped: u64,
    /// Input tokens over every answered row. Only input tokens are billed.
    pub(crate) input_tokens: u64,
    /// The estimated cost in US dollars, or `null` when an answering model's price is not known.
    pub(crate) cost_usd: Option<f64>,
    /// From the first row read to the last record written, in milliseconds.
    pub(crate) wall_time_ms: u64,
    /// Rows answered or failed per second of wall time.
    pub(crate) rows_per_second: f64,
    /// API calls retried, over every row.
    pub(crate) retries: u64,
    /// The versioned ids of the models that answered, in the order first seen.
    pub(crate) models: Vec<String>,
    /// Why the run stopped sending early, or `null` when it went through the input.
    pub(crate) stopped_by: Option<StopReason>,
}

impl Summary {
    /// Fills in the figures that depend on the whole run.
    pub(crate) fn finish(&mut self, elapsed: Duration) {
        self.skipped = self.rows_total.saturating_sub(self.ok + self.failed);
        self.wall_time_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        let seconds = elapsed.as_secs_f64();
        #[allow(clippy::cast_precision_loss)] // A row count is far below 2^52.
        let done = (self.ok + self.failed) as f64;
        self.rows_per_second = if seconds > 0.0 {
            (done / seconds * 10.0).round() / 10.0
        } else {
            0.0
        };
    }

    /// The summary for a person, on stderr.
    pub(crate) fn human(&self, ui: Ui) -> String {
        let mut text = format!(
            "{} {} rows: {} ok, {} failed, {} skipped",
            ui.bold("batch:"),
            self.rows_total,
            self.ok,
            self.failed,
            self.skipped
        );
        if let Some(reason) = self.stopped_by {
            let _ = write!(
                text,
                " ({})",
                match reason {
                    StopReason::FailFast => "stopped by --fail-fast",
                    StopReason::MaxErrors => "stopped by --max-errors",
                    StopReason::InvalidInput => "stopped at a row that cannot be used",
                    StopReason::OutputClosed => "stopped: the output was closed",
                }
            );
        }
        let facts = [
            format!("{} input tokens", self.input_tokens),
            self.cost_usd.map_or_else(
                || "cost unknown".to_owned(),
                |cost| format!("est. cost ${cost:.6}"),
            ),
            format!(
                "{:.1} s",
                Duration::from_millis(self.wall_time_ms).as_secs_f64()
            ),
            format!("{} rows/s", self.rows_per_second),
            format!(
                "{} retr{}",
                self.retries,
                if self.retries == 1 { "y" } else { "ies" }
            ),
        ];
        let _ = write!(text, "\n  {}", ui.dim(&facts.join(ui.separator())));
        if !self.models.is_empty() {
            let _ = write!(
                text,
                "\n  {}",
                ui.dim(&format!("model {}", self.models.join(", ")))
            );
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{StopReason, Summary};
    use crate::output::Ui;

    #[test]
    fn a_finished_summary_counts_the_skipped_rows_and_the_throughput() {
        let mut summary = Summary {
            rows_total: 10,
            ok: 5,
            failed: 1,
            input_tokens: 1500,
            cost_usd: Some(0.000_063),
            retries: 1,
            models: vec!["jev-1.13.0".to_owned()],
            stopped_by: Some(StopReason::FailFast),
            ..Summary::default()
        };

        summary.finish(Duration::from_millis(2000));

        assert_eq!(summary.skipped, 4);
        assert!((summary.rows_per_second - 3.0).abs() < f64::EPSILON);
        assert_eq!(
            summary.human(Ui::plain()),
            "batch: 10 rows: 5 ok, 1 failed, 4 skipped (stopped by --fail-fast)\n  \
             1500 input tokens | est. cost $0.000063 | 2.0 s | 3 rows/s | 1 retry\n  model jev-1.13.0"
        );
        let json = serde_json::to_value(&summary).unwrap();
        assert_eq!(json["stopped_by"], "fail_fast");
        assert_eq!(json["wall_time_ms"], 2000);
    }
}
