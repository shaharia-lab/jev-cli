//! Hooks for `jev`'s own test suite. Compiled only with the `internal-test-hooks` feature, which
//! no release build enables.
//!
//! They exist because the behaviour they reach is real and must be tested from outside the
//! process (what goes to stdout, what goes to stderr, the exit code) before any command that would
//! reach it naturally has been written.

use std::time::Duration;

use clap::{Args, Subcommand, ValueEnum};
use jev_client::{Error, ErrorKind, Response};
use serde_json::Value;

use super::Context;
use crate::error::CliError;
use crate::output::ResultEnvelope;

#[derive(Debug, Subcommand)]
pub(crate) enum DebugCommand {
    /// Render an API response read from stdin, as `jev eval` will
    Render(RenderArgs),
    /// Fail with the error a transport would return
    Error(ErrorArgs),
    /// Reach a point where a value would be prompted for
    Prompt,
    /// Panic
    Panic,
}

#[derive(Debug, Args)]
pub(crate) struct RenderArgs {
    /// Read the response from this file instead of stdin (a terminal test has no stdin to pipe)
    #[arg(long)]
    file: Option<std::path::PathBuf>,
    /// The model name the request used
    #[arg(long, default_value = "jev-latest")]
    requested_model: String,
    /// The request id to report
    #[arg(long)]
    request_id: Option<String>,
    /// The latency to report, in milliseconds
    #[arg(long, default_value_t = 0)]
    latency_ms: u64,
}

#[derive(Debug, Args)]
pub(crate) struct ErrorArgs {
    /// The kind of error
    #[arg(long, value_enum)]
    kind: Kind,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum Kind {
    Authentication,
    PermissionDenied,
    Unprocessable,
    NotFound,
    RateLimit,
    Overloaded,
    Server,
    Timeout,
    Connection,
    InvalidResponse,
}

pub(crate) fn run(command: &DebugCommand, context: &mut Context<'_>) -> Result<(), CliError> {
    match command {
        DebugCommand::Render(arguments) => render(arguments, context),
        DebugCommand::Error(arguments) => Err(synthetic(arguments.kind).into()),
        DebugCommand::Prompt => context.interaction.ensure_can_prompt(
            "the API key",
            "pipe the key to `jev auth login --with-token`, or set TYPESAFE_API_KEY",
        ),
        // The point of this hook is to panic, so that the crash path can be tested.
        #[allow(clippy::panic)]
        DebugCommand::Panic => panic!("forced by `jev debug panic`"),
    }
}

fn render(arguments: &RenderArgs, context: &mut Context<'_>) -> Result<(), CliError> {
    let text = if let Some(path) = &arguments.file {
        std::fs::read_to_string(path).map_err(|error| {
            CliError::usage(format!("could not read {}: {error}", path.display()))
        })?
    } else {
        let mut text = String::new();
        context.stdin.read_to_string(&mut text).map_err(|error| {
            CliError::usage(format!("could not read a response from stdin: {error}"))
        })?;
        text
    };
    let raw: Value = serde_json::from_str(&text)
        .map_err(|error| CliError::usage(format!("the response is not JSON: {error}")))?;
    let response: Response = serde_json::from_value(raw.clone())
        .map_err(|error| CliError::usage(format!("that is not an API response: {error}")))?;

    let envelope = ResultEnvelope::new(
        &response,
        Some(&raw),
        &arguments.requested_model,
        arguments.request_id.clone(),
        Duration::from_millis(arguments.latency_ms),
        None,
    );
    context.output.emit(&envelope, context.stdout)
}

fn synthetic(kind: Kind) -> Error {
    let (kind, status, error_type, message) = match kind {
        Kind::Authentication => (
            ErrorKind::Authentication,
            Some(401),
            "authentication_error",
            "Invalid API key.",
        ),
        Kind::PermissionDenied => (
            ErrorKind::PermissionDenied,
            Some(403),
            "permission_error",
            "Not allowed.",
        ),
        Kind::Unprocessable => (
            ErrorKind::Unprocessable,
            Some(422),
            "invalid_request_error",
            "state: Field required",
        ),
        Kind::NotFound => (
            ErrorKind::NotFound,
            Some(404),
            "not_found_error",
            "No such model.",
        ),
        Kind::RateLimit => (
            ErrorKind::RateLimit,
            Some(429),
            "rate_limit_error",
            "Slow down.",
        ),
        Kind::Overloaded => (
            ErrorKind::Overloaded,
            Some(529),
            "overloaded_error",
            "Overloaded.",
        ),
        Kind::Server => (ErrorKind::Server, Some(500), "internal_error", "Boom."),
        Kind::Timeout => (ErrorKind::Timeout, None, "", "no response within 30s"),
        Kind::Connection => (ErrorKind::Connection, None, "", "connection refused"),
        Kind::InvalidResponse => (
            ErrorKind::InvalidResponse,
            Some(200),
            "",
            "the body is not valid JSON (line 1, column 1)",
        ),
    };
    let mut error = Error::new(kind, message).with_request_id(Some("req_test".to_owned()));
    if let Some(status) = status {
        error = error.with_status(status);
    }
    if !error_type.is_empty() {
        error = error.with_error_type(error_type);
    }
    if error.is_retryable() {
        error.with_attempts(3)
    } else {
        error.with_attempts(1)
    }
}
