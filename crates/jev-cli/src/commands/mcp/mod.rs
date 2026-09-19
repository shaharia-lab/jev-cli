//! `jev mcp serve`: jev's evaluations as Model Context Protocol tools, over stdio.
//!
//! The protocol is JSON-RPC 2.0, one message per line. A server needs only a handful of methods
//! (`initialize`, `ping`, `tools/list`, `tools/call`), so they are implemented here rather than
//! through an SDK: that keeps the dependency tree small and keeps one loop in charge of stdout,
//! which must carry nothing but protocol messages. Logs, notices and warnings go to stderr.
//!
//! Every tool that evaluates goes through [`crate::evaluate::prepare`] and
//! [`crate::evaluate::send`], like the commands do. The tools themselves are in [`tools`].

mod tools;

use std::io::{BufRead, BufReader, ErrorKind};

use jev_client::pricing::{self, Price};
use jev_client::validate::{Document, Finding};
use jev_client::{Transport, Usage};
use serde::Serialize;
use serde_json::{Map, Value, json};

use super::Context;
use crate::cli::McpServeArgs;
use crate::config::Settings;
use crate::error::CliError;
use crate::evaluate::{self, Evaluation, PrepareOptions, Prepared};

use self::tools::Tool;

/// Protocol versions this server speaks, newest first. A client asking for another version is
/// offered the newest, and decides for itself whether to go on.
const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// What the server tells a client about Jev when it connects, for the model that will use it.
const INSTRUCTIONS: &str = "Jev (TypeSafe AI) reads a `state` (text or JSON) and answers typed \
questions with calibrated probabilities; it never generates text. Put every question about one \
state into a single `evaluate` call: questions are answered in parallel, and one call is far \
cheaper than several. Use `noul`, `choice` or `score` for a single question, `validate` to check a \
request offline for free, and `list_models` to see model names. Jev cannot count, do arithmetic or \
compare numbers, and reads dates as text: keep those in code. Always give a choice an escape \
option such as `other` or `not_stated`. Ask one snap judgment per question.";

/// JSON-RPC error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// Serves MCP on stdin and stdout until the client closes stdin.
///
/// # Errors
///
/// A usage error when the configuration cannot be loaded, or an internal error when stdin or
/// stdout fails. Nothing that goes wrong inside a tool call ends the server.
pub(crate) fn serve(arguments: &McpServeArgs, context: &mut Context<'_>) -> Result<(), CliError> {
    let settings = context.settings()?.clone();
    let price_override = context.config_file()?.pricing_usd_per_mtok;
    // Read once: a server without a usable key still starts, so that `validate` works and every
    // other tool can say what is wrong. The failure is returned by each call that needs the API.
    let transport = context.transport(&settings);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            CliError::internal(format!("could not start the async runtime: {error}"))
        })?;
    let mut server = Server {
        session: Session {
            settings,
            transport,
            price_override,
            max_cost_usd_per_call: arguments.max_cost_usd_per_call,
            spend: Spend::default(),
            runtime,
        },
        tools: tools::all(),
    };
    tracing::debug!("serving MCP over stdio");

    let mut input = BufReader::new(&mut *context.stdin);
    let stdout = &mut *context.stdout;
    let mut line = Vec::new();
    loop {
        line.clear();
        match input.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(CliError::internal(format!("could not read stdin: {error}")));
            }
        }
        let Some(reply) = server.handle(&line) else {
            continue;
        };
        let written = writeln!(stdout, "{reply}").and_then(|()| stdout.flush());
        match written {
            Ok(()) => {}
            // The client has gone away: there is nobody left to serve.
            Err(error) if error.kind() == ErrorKind::BrokenPipe => break,
            Err(error) => {
                return Err(CliError::internal(format!(
                    "could not write to stdout: {error}"
                )));
            }
        }
    }
    tracing::debug!("the client closed stdin; stopping");
    Ok(())
}

/// The protocol side: messages in, messages out.
struct Server<T> {
    session: Session<T>,
    tools: Vec<Tool<T>>,
}

impl<T: Transport> Server<T> {
    /// Answers one line from the client. Notifications, and lines with nothing on them, get no
    /// answer.
    fn handle(&mut self, line: &[u8]) -> Option<Value> {
        if line.iter().all(u8::is_ascii_whitespace) {
            return None;
        }
        let Ok(message) = serde_json::from_slice::<Value>(line) else {
            return Some(failure(&Value::Null, PARSE_ERROR, "Parse error"));
        };
        let Some(message) = message.as_object() else {
            // Batches were removed from the protocol in 2025-06-18.
            return Some(failure(
                &Value::Null,
                INVALID_REQUEST,
                "Invalid Request: expected one JSON-RPC object per line",
            ));
        };
        let method = message.get("method").and_then(Value::as_str);
        let Some(id) = message.get("id") else {
            // A notification (`notifications/initialized`, `notifications/cancelled`, ...) or a
            // response to a request this server never sends. Neither is answered.
            tracing::debug!(method, "MCP notification");
            return None;
        };
        let Some(method) = method else {
            return Some(failure(
                id,
                INVALID_REQUEST,
                "Invalid Request: `method` is missing",
            ));
        };
        tracing::debug!(method, "MCP request");
        let params = message.get("params");
        let outcome = match method {
            "initialize" => Ok(initialize(params)),
            "ping" => Ok(json!({})),
            "tools/list" => {
                Ok(json!({ "tools": self.tools.iter().map(Tool::describe).collect::<Vec<_>>() }))
            }
            "tools/call" => self.call(params),
            _ => Err((METHOD_NOT_FOUND, format!("Method not found: `{method}`"))),
        };
        Some(match outcome {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => failure(id, code, &message),
        })
    }

    /// Runs a tool. A tool that fails answers with a tool error, which the model gets to read; only
    /// a call that names no known tool is a protocol error.
    fn call(&mut self, params: Option<&Value>) -> Result<Value, (i64, String)> {
        let name = params
            .and_then(|params| params.get("name"))
            .and_then(Value::as_str)
            .ok_or((
                INVALID_PARAMS,
                "Invalid params: `name` is missing".to_owned(),
            ))?;
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.name == name)
            .ok_or_else(|| (INVALID_PARAMS, format!("Unknown tool: `{name}`")))?;
        let empty = Map::new();
        let result = match params.and_then(|params| params.get("arguments")) {
            None | Some(Value::Null) => (tool.call)(&mut self.session, &empty),
            Some(Value::Object(arguments)) => (tool.call)(&mut self.session, arguments),
            Some(_) => Err(
                CliError::usage("the tool's arguments must be a JSON object")
                    .hint("pass the arguments as an object, as the tool's input schema describes"),
            ),
        };
        Ok(match result {
            Ok(value) => json!({
                "content": [{ "type": "text", "text": value.to_string() }],
                "structuredContent": value,
                "isError": false
            }),
            Err(error) => {
                tracing::debug!(tool = name, code = error.code, "MCP tool error");
                let value = error.to_json();
                json!({
                    "content": [{ "type": "text", "text": value.to_string() }],
                    "structuredContent": value,
                    "isError": true
                })
            }
        })
    }
}

/// The answer to `initialize`.
fn initialize(params: Option<&Value>) -> Value {
    let requested = params
        .and_then(|params| params.get("protocolVersion"))
        .and_then(Value::as_str);
    let version = requested
        .filter(|version| PROTOCOL_VERSIONS.contains(version))
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": {
            "name": "jev",
            "title": "jev (unofficial CLI for TypeSafe AI's Jev)",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": INSTRUCTIONS
    })
}

/// A JSON-RPC error response.
fn failure(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// What lasts between tool calls: the settings read at start-up, the API connection, and the
/// money spent so far.
struct Session<T> {
    settings: Settings,
    transport: Result<T, CliError>,
    /// `[pricing] usd_per_mtok` from the configuration, which replaces the built-in prices.
    price_override: Option<f64>,
    max_cost_usd_per_call: Option<f64>,
    spend: Spend,
    runtime: tokio::runtime::Runtime,
}

impl<T: Transport> Session<T> {
    /// Validates, checks the guardrails, and sends: the path every evaluating tool takes.
    fn evaluate(&mut self, document: Document) -> Result<Evaluated, CliError> {
        let prepared = evaluate::prepare(document, &self.settings, &PrepareOptions::default())?;
        self.check_cost(&prepared)?;
        let transport = self.transport.as_ref().map_err(Clone::clone)?;
        let evaluation =
            self.runtime
                .block_on(evaluate::send(transport, &prepared, self.price_override))?;
        self.spend.record(evaluation.envelope.cost_usd);
        Ok(Evaluated {
            evaluation,
            warnings: prepared.report.warnings().cloned().collect(),
        })
    }

    /// Refuses a call whose estimated cost is above `--max-cost-usd-per-call`, or cannot be known.
    fn check_cost(&self, prepared: &Prepared) -> Result<(), CliError> {
        let Some(limit) = self.max_cost_usd_per_call else {
            return Ok(());
        };
        let model = &prepared.requested_model;
        // An alias has no price: what it points at moves. Without a price the limit cannot be
        // checked, and a guardrail that cannot be checked refuses rather than lets a call through.
        let price = self
            .price_override
            .and_then(Price::from_usd_per_mtok)
            .or_else(|| pricing::price_for(model))
            .ok_or_else(|| {
                CliError::cost_limit(format!(
                    "`{model}` has no known price, so this call cannot be checked against --max-cost-usd-per-call; nothing was sent"
                ))
                .hint("use a versioned model id with a known price, such as `jev-1.13.0`: pass `model`, or start the server with --model; or set `usd_per_mtok` under `[pricing]` in config.toml")
            })?;
        let tokens = prepared
            .report
            .size
            .as_ref()
            .map(|size| size.total_tokens)
            .ok_or_else(|| {
                CliError::cost_limit(
                    "the size of this request could not be estimated, so it cannot be checked against --max-cost-usd-per-call; nothing was sent",
                )
                .hint("give the request a `state`")
            })?;
        let estimate = price.estimate_usd(Usage::new(tokens, 0));
        if estimate <= limit {
            return Ok(());
        }
        let mut error = CliError::cost_limit(format!(
            "this call is estimated at ${estimate:.6} ({tokens} input tokens), above the limit of ${limit:.6} per call; nothing was sent"
        ))
        .hint("trim the state or ask fewer questions, or restart the server with a higher --max-cost-usd-per-call");
        error.details = Some(json!({
            "estimated_cost_usd": estimate,
            "estimated_input_tokens": tokens,
            "max_cost_usd_per_call": limit
        }));
        Err(error)
    }

    /// The spend counter as a tool result shows it.
    fn spend(&self) -> Value {
        json!({
            "calls": self.spend.calls,
            "estimated_cost_usd": self.spend.estimated_cost_usd,
            "unpriced_calls": self.spend.unpriced_calls,
            "max_cost_usd_per_call": self.max_cost_usd_per_call
        })
    }
}

/// A successful evaluation and the validation warnings its request had.
struct Evaluated {
    evaluation: Evaluation,
    warnings: Vec<Finding>,
}

/// An evaluating tool's result: `result` (the envelope, as the matching command prints it with
/// `-o json`), then the validation warnings when there are any, then the session's spend so far.
fn with_session(
    result: &impl Serialize,
    warnings: &[Finding],
    spend: Value,
) -> Result<Value, CliError> {
    let mut value = serde_json::to_value(result)
        .map_err(|error| CliError::internal(format!("could not encode a result: {error}")))?;
    if let Value::Object(object) = &mut value {
        if !warnings.is_empty() {
            object.insert("warnings".to_owned(), json!(warnings));
        }
        object.insert("session".to_owned(), spend);
    }
    Ok(value)
}

/// Money spent in this session, as estimated from the usage each answer reported.
#[derive(Debug, Default)]
struct Spend {
    /// Evaluations the API answered.
    calls: u64,
    /// The sum of their estimated costs, in US dollars.
    estimated_cost_usd: f64,
    /// Evaluations whose cost could not be estimated, because the model's price is not known.
    unpriced_calls: u64,
}

impl Spend {
    fn record(&mut self, cost_usd: Option<f64>) {
        self.calls += 1;
        match cost_usd {
            Some(cost) => self.estimated_cost_usd += cost,
            None => self.unpriced_calls += 1,
        }
    }
}
