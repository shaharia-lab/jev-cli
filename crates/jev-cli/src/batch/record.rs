//! One line of `jev batch run` output. `jev schema batch-record` prints the schema of [`Record`].

use indexmap::IndexMap;
use jev_client::{Answer, Usage};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::Serialize;
use serde_json::Value;

use crate::error::{CliError, ErrorBody};
use crate::output::ResultEnvelope;

/// The record of one row: what it was evaluated to, or why it failed.
#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
#[schemars(
    description = "One line of `jev batch run` output: the result of one input row. `status` says which form it is."
)]
pub(crate) enum Record<'a> {
    /// A row that was evaluated.
    Ok(Evaluated<'a>),
    /// A row that failed. The run goes on, and exits 7 at the end.
    Failed(Failed<'a>),
}

impl<'a> Record<'a> {
    /// The record of a row that was evaluated.
    pub(crate) fn ok(id: &'a Value, envelope: &'a ResultEnvelope) -> Self {
        Self::Ok(Evaluated {
            id,
            status: OkStatus::Ok,
            model: &envelope.model,
            answers: &envelope.answers,
            usage: envelope.usage,
            cost_usd: envelope.cost_usd,
            request_id: envelope.request_id.as_deref(),
            latency_ms: envelope.latency_ms,
        })
    }

    /// The record of a row that failed.
    pub(crate) fn failed(id: &'a Value, error: &'a CliError) -> Self {
        Self::Failed(Failed {
            id,
            status: FailedStatus::Error,
            error: ErrorBody::from(error),
        })
    }
}

/// The record of a row that was evaluated: its id, then the same facts `jev eval` reports.
#[derive(Serialize, JsonSchema)]
// Every field is always present, `null` when it has no value.
#[schemars(extend("required" = [
    "id", "status", "model", "answers", "usage", "cost_usd", "request_id", "latency_ms"
]))]
pub(crate) struct Evaluated<'a> {
    /// The row's id: the value of --id-field (a string or a number), else its 1-based line number.
    #[schemars(schema_with = "id_schema")]
    id: &'a Value,
    /// Always `ok`.
    status: OkStatus,
    /// The versioned id of the model that answered, such as `jev-1.13.0`.
    model: &'a str,
    /// One answer per question, exactly as the API returned them.
    #[schemars(with = "IndexMap<String, Answer>")]
    answers: &'a IndexMap<String, Value>,
    /// Token usage. Only input tokens are billed.
    usage: Usage,
    /// The estimated cost in US dollars, or `null` when the model's price is not known.
    cost_usd: Option<f64>,
    /// The `x-typesafe-request-id` to quote when asking TypeSafe about this row.
    request_id: Option<&'a str>,
    /// Wall-clock time for the row's call, including retries, in milliseconds.
    latency_ms: u64,
}

/// The record of a row that failed: its id and the same error object `jev` prints on stderr.
#[derive(Serialize, JsonSchema)]
pub(crate) struct Failed<'a> {
    /// The row's id, as in an `ok` record; `null` only if jev itself failed while handling it.
    #[schemars(schema_with = "failed_id_schema")]
    id: &'a Value,
    /// Always `error`.
    status: FailedStatus,
    /// What went wrong: the same object as a JSON error on stderr (see `jev schema error`).
    error: ErrorBody<'a>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum OkStatus {
    Ok,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum FailedStatus {
    Error,
}

fn id_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": ["string", "number"] })
}

fn failed_id_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": ["string", "number", "null"] })
}
