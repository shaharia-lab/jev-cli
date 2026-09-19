//! `--dry-run`: every row checked and priced as a run would send it, and nothing sent.

use std::fmt::Write as _;

use jev_client::Usage;
use jev_client::pricing::{self, Price};
use serde::Serialize;

use super::engine::Job;
use super::{Ids, Keyed, Row, RowProblem};
use crate::evaluate::{self, PrepareOptions};
use crate::output::{Render, Ui};

/// How many problems the plan lists before counting the rest.
const MAX_LISTED_PROBLEMS: usize = 100;

/// What a run would do. This shape is versioned public API.
#[derive(Debug, Serialize)]
pub(crate) struct Plan {
    /// Always `true`: nothing was sent, and nothing was billed.
    dry_run: bool,
    /// Rows read, up to --limit.
    pub(crate) rows_total: u64,
    /// Rows that would be sent: one request each, before any retry.
    pub(crate) requests: u64,
    /// Rows that would not be sent because the output already records them `ok` (`--resume`).
    pub(crate) already_ok: u64,
    /// Rows that cannot be used, or whose request fails validation.
    pub(crate) invalid_rows: u64,
    /// Estimated input tokens over every request, or `null` with --skip-size-check.
    pub(crate) estimated_input_tokens: Option<u64>,
    /// The estimated cost in US dollars, or `null` when the model has no known price (an alias
    /// such as `jev-latest` never has one) or the size was not estimated.
    pub(crate) estimated_cost_usd: Option<f64>,
    /// The model name the requests would carry, which may be an alias.
    pub(crate) requested_model: String,
    /// What is wrong with the invalid rows, by line; at most 100.
    pub(crate) problems: Vec<RowProblem>,
}

impl Plan {
    /// Checks and prices every row, the way [`super::run`] would send it.
    pub(crate) fn new(
        rows: impl IntoIterator<Item = Result<Row, RowProblem>>,
        job: &Job<'_>,
        requested_model: &str,
    ) -> Self {
        let mut plan = Self {
            dry_run: true,
            rows_total: 0,
            requests: 0,
            already_ok: 0,
            invalid_rows: 0,
            estimated_input_tokens: (!job.skip_size_check).then_some(0),
            estimated_cost_usd: None,
            requested_model: requested_model.to_owned(),
            problems: Vec::new(),
        };
        let mut ids = job.mapping.id_field.is_some().then(Ids::default);
        for row in rows {
            plan.rows_total += 1;
            if let Err(problem) = plan.check(row, job, &mut ids) {
                plan.invalid_rows += 1;
                if plan.problems.len() < MAX_LISTED_PROBLEMS {
                    plan.problems.push(problem);
                }
            }
        }
        let price = job
            .usd_per_mtok
            .and_then(Price::from_usd_per_mtok)
            .or_else(|| pricing::price_for(requested_model));
        plan.estimated_cost_usd = match (price, plan.estimated_input_tokens) {
            (Some(price), Some(tokens)) => {
                Some((price.estimate_usd(Usage::new(tokens, 0)) * 1e9).round() / 1e9)
            }
            _ => None,
        };
        plan
    }

    fn check(
        &mut self,
        row: Result<Row, RowProblem>,
        job: &Job<'_>,
        ids: &mut Option<Ids>,
    ) -> Result<(), RowProblem> {
        let row = row?;
        let line = row.line;
        let Keyed { id, state } = job.mapping.key(row)?;
        if let Some(ids) = ids {
            ids.insert(&id, line)?;
        }
        if job.done.is_some_and(|done| done.contains(&id)) {
            self.already_ok += 1;
            return Ok(());
        }
        let options = PrepareOptions {
            state: Some(state),
            strict: job.strict,
            skip_size_check: job.skip_size_check,
        };
        let prepared =
            evaluate::prepare(job.questions.clone(), job.settings, &options).map_err(|error| {
                let findings: Vec<&str> = error
                    .lines
                    .iter()
                    .filter_map(|line| line.strip_prefix("- "))
                    .collect();
                let message = if findings.is_empty() {
                    error.message.clone()
                } else {
                    format!("the request is not valid: {}", findings.join("; "))
                };
                RowProblem::at(line, message)
            })?;
        self.requests += 1;
        if let (Some(total), Some(size)) = (&mut self.estimated_input_tokens, &prepared.report.size)
        {
            *total += size.total_tokens;
        }
        Ok(())
    }
}

impl Render for Plan {
    fn human(&self, ui: Ui) -> String {
        let mut text = format!(
            "{}\n{} rows: {} to send, {} already ok, {} invalid\n",
            ui.bold("dry run: nothing was sent"),
            self.rows_total,
            self.requests,
            self.already_ok,
            self.invalid_rows
        );
        match self.estimated_input_tokens {
            Some(tokens) => {
                let _ = writeln!(text, "estimated input tokens: {tokens}");
            }
            None => text.push_str("estimated input tokens: not estimated (--skip-size-check)\n"),
        }
        match self.estimated_cost_usd {
            Some(cost) => {
                let _ = writeln!(
                    text,
                    "estimated cost: ${cost:.6} on {}",
                    self.requested_model
                );
            }
            None if self.estimated_input_tokens.is_none() => {
                text.push_str("estimated cost: unknown without a size estimate\n");
            }
            None => {
                let _ = writeln!(
                    text,
                    "estimated cost: unknown for `{}`; a versioned model id such as `jev-1.13.0` has a price",
                    self.requested_model
                );
            }
        }
        for problem in &self.problems {
            let _ = writeln!(text, "- {}", problem.describe());
        }
        text
    }
}
