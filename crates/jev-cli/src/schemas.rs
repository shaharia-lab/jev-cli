//! The JSON Schemas of what `jev` reads and writes, generated from the types that do the reading
//! and writing, so that a schema cannot say something the code does not do.
//!
//! `jev schema` prints them and `jev mcp serve` builds its tool input schemas from them.

use jev_client::{Answer, Request};
use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde_json::{Map, Value, json};

use crate::batch::Record;
use crate::error::ErrorDocument;
use crate::output::ResultEnvelope;

/// A type's JSON Schema (draft 2020-12) as a plain object, with every definition inlined: some
/// MCP clients do not follow `$ref`, and a self-contained schema is simpler to publish.
pub(crate) fn inlined<T: JsonSchema>() -> Map<String, Value> {
    let mut settings = SchemaSettings::draft2020_12();
    settings.inline_subschemas = true;
    match settings
        .into_generator()
        .into_root_schema_for::<T>()
        .to_value()
    {
        Value::Object(schema) => schema,
        _ => Map::new(),
    }
}

/// A complete request: the body of `POST /v1/systemone`, and a request file that needs nothing
/// else to be evaluated.
pub(crate) fn request() -> Value {
    published(
        inlined::<Request>(),
        "jev request",
        "A complete evaluation request: `state`, `model` and `questions`. This is the body of \
         `POST /v1/systemone` exactly, and a request file `jev eval -f` can send as it is. A file \
         that leaves out `state` or `model` follows the `questions` schema instead.",
    )
}

/// A question set: a request file whose state, and possibly model, come from somewhere else.
pub(crate) fn questions() -> Value {
    let mut schema = inlined::<Request>();
    schema.insert("required".to_owned(), json!(["questions"]));
    published(
        schema,
        "jev question set",
        "A question set: a request file in which only `questions` is required. `state` comes from \
         --state, --state-file or stdin (`jev eval`) or from each input row (`jev batch run`). \
         The model is --model, else `model` here, else TYPESAFE_DEFAULT_MODEL, the profile's \
         model, and finally `jev-latest`. `jev validate` checks files of this shape.",
    )
}

/// What `jev eval`, `jev noul`, `jev choice` and `jev score` print on stdout with `-o json`.
pub(crate) fn output() -> Value {
    let envelope = inlined::<ResultEnvelope>();
    let answer = inlined::<Answer>();

    // The shortcuts replace `answers` with the fields of their one answer, at the top level.
    let mut single = Map::new();
    single.insert("type".to_owned(), json!("object"));
    let mut properties = envelope
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    properties.shift_remove("answers");
    single.insert("properties".to_owned(), Value::Object(properties));
    let required: Vec<Value> = envelope
        .get("required")
        .and_then(Value::as_array)
        .map(|required| {
            required
                .iter()
                .filter(|name| name.as_str() != Some("answers"))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    single.insert("required".to_owned(), Value::Array(required));
    let one_answer = without_root_keywords(answer);

    let mut many = without_root_keywords(envelope);
    many.insert(
        "title".to_owned(),
        json!("jev eval: every answer under `answers`"),
    );
    let single = json!({
        "title": "jev noul|choice|score: the one answer's fields at the top level",
        "allOf": [Value::Object(single), Value::Object(one_answer)]
    });

    let mut schema = Map::new();
    schema.insert("anyOf".to_owned(), json!([Value::Object(many), single]));
    published(
        schema,
        "jev result",
        "The result of an evaluation, as `jev eval`, `jev noul`, `jev choice` and `jev score` \
         print it with `-o json` (and `jev mcp serve` returns it). `jev eval` puts every answer \
         under `answers`, keyed by question id, exactly as the API returned it; the one-question \
         commands put their answer's own fields (`type`, `noul`, `choice`, `score`, ...) at the \
         top level instead. `gate` is present only when a gating flag was used. New fields may \
         be added; existing ones do not change within a major version.",
    )
}

/// One line of `jev batch run` output.
pub(crate) fn batch_record() -> Value {
    published(
        inlined::<Record<'_>>(),
        "jev batch record",
        "One line of `jev batch run` output, one per input row, in completion order. `status` \
         `ok`: the row's `id` and the same facts `jev eval -o json` reports (`model`, `answers`, \
         `usage`, `cost_usd`, `request_id`, `latency_ms`). `status` `error`: the row's `id` and \
         an `error` object shaped like the JSON error on stderr. A run with any `error` record \
         exits 7.",
    )
}

/// The single JSON object `jev` prints on stderr when a command fails and the output format is
/// machine-readable.
pub(crate) fn error() -> Value {
    published(
        inlined::<ErrorDocument<'_>>(),
        "jev error",
        "The single JSON object `jev` prints on stderr when a command fails and the output format \
         is machine-readable (`-o json`, `-o yaml`, `-o jsonl`, or stdout not a terminal), and \
         the error a `jev mcp serve` tool call returns. `code` is a stable name; `exit_code` is \
         the process exit code. Every field is always present.",
    )
}

/// Gives a schema the keywords a standalone published schema carries, in a stable order.
fn published(schema: Map<String, Value>, title: &str, description: &str) -> Value {
    let mut root = Map::new();
    root.insert(
        "$schema".to_owned(),
        json!("https://json-schema.org/draft/2020-12/schema"),
    );
    root.insert("title".to_owned(), json!(title));
    root.insert("description".to_owned(), json!(description));
    let mut schema = without_root_keywords(schema);
    schema.shift_remove("description");
    root.extend(schema);
    Value::Object(root)
}

/// A schema without the keywords that only belong at the root of a document.
fn without_root_keywords(mut schema: Map<String, Value>) -> Map<String, Value> {
    for keyword in ["$schema", "title"] {
        schema.shift_remove(keyword);
    }
    schema
}
