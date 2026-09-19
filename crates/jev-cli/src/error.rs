//! The one error type of the binary, and the single place where errors become exit codes.

use std::fmt::Write as _;

use jev_client::ErrorKind;
use serde::Serialize;
use serde_json::{Value, json};

use crate::exit::Exit;
use crate::output::Ui;

/// Where bugs are reported.
pub(crate) const ISSUES_URL: &str = "https://github.com/shaharia-lab/jev-cli/issues";

/// An error a command ends with.
///
/// Every error names the next action in its `hint`, because the reader is as likely to be an AI
/// agent as a person. `code` is a stable, machine-readable name; `exit` is the process exit code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CliError {
    pub(crate) code: &'static str,
    pub(crate) exit: Exit,
    pub(crate) message: String,
    pub(crate) hint: Option<String>,
    pub(crate) error_type: Option<String>,
    pub(crate) request_id: Option<String>,
    pub(crate) http_status: Option<u16>,
    pub(crate) retryable: bool,
    /// Structured detail for a program, such as the findings of a failed validation.
    pub(crate) details: Option<Value>,
    /// Extra lines for a person, shown between the message and the hint.
    pub(crate) lines: Vec<String>,
}

impl CliError {
    fn new(code: &'static str, exit: Exit, message: impl Into<String>) -> Self {
        Self {
            code,
            exit,
            message: message.into(),
            hint: None,
            error_type: None,
            request_id: None,
            http_status: None,
            retryable: false,
            details: None,
            lines: Vec::new(),
        }
    }

    /// The command line, a flag value or a request file is wrong.
    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self::new("usage", Exit::Usage, message)
    }

    /// There is no usable API key.
    pub(crate) fn auth(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(code, Exit::Auth, message)
    }

    /// A spending guardrail refused a call before anything was sent.
    pub(crate) fn cost_limit(message: impl Into<String>) -> Self {
        Self::new("cost_limit", Exit::Usage, message)
    }

    /// Something that should never happen did.
    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", Exit::Internal, message).hint(format!(
            "this is a bug; please report it at {ISSUES_URL} with the output of `jev version`"
        ))
    }

    /// A command that exists in the command tree but has no behaviour yet.
    pub(crate) fn not_implemented(command: &str, issue: u32) -> Self {
        Self::new(
            "not_implemented",
            Exit::Usage,
            format!("`jev {command}` is not implemented yet"),
        )
        .hint(format!(
            "it is planned for the first release; progress is tracked at {ISSUES_URL}/{issue}"
        ))
    }

    /// A value would have to be asked for interactively, and that is not allowed right now.
    // First used outside the test hooks by `jev auth login` (issue #10).
    #[cfg_attr(not(feature = "internal-test-hooks"), allow(dead_code))]
    pub(crate) fn prompt_not_allowed(what: &str, reason: &str, instead: &str) -> Self {
        Self::new(
            "input_required",
            Exit::Usage,
            format!("cannot ask for {what}: {reason}"),
        )
        .hint(instead)
    }

    /// Sets the next action.
    pub(crate) fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// The single JSON object printed on stderr when the output format is machine-readable.
    pub(crate) fn to_json(&self) -> Value {
        json!({ "error": ErrorBody::from(self) })
    }

    /// The text printed on stderr for a person.
    pub(crate) fn to_human(&self, ui: Ui) -> String {
        let mut text = format!("{} {}", ui.error_label("error:"), self.message);
        for line in &self.lines {
            let _ = write!(text, "\n  {line}");
        }
        if let Some(hint) = &self.hint {
            let _ = write!(text, "\n  {} {hint}", ui.dim("hint:"));
        }
        if let Some(request_id) = &self.request_id {
            let _ = write!(text, "\n  {} {request_id}", ui.dim("request id:"));
        }
        text
    }
}

/// The documented shape of a JSON error. Every field is always present, so a reader never has to
/// test for one.
#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    exit_code: u8,
    error_type: Option<&'a str>,
    message: &'a str,
    hint: Option<&'a str>,
    request_id: Option<&'a str>,
    http_status: Option<u16>,
    retryable: bool,
    details: Option<&'a Value>,
}

impl<'a> From<&'a CliError> for ErrorBody<'a> {
    fn from(error: &'a CliError) -> Self {
        Self {
            code: error.code,
            exit_code: error.exit.code(),
            error_type: error.error_type.as_deref(),
            message: &error.message,
            hint: error.hint.as_deref(),
            request_id: error.request_id.as_deref(),
            http_status: error.http_status,
            retryable: error.retryable,
            details: error.details.as_ref(),
        }
    }
}

impl From<jev_client::Error> for CliError {
    fn from(error: jev_client::Error) -> Self {
        let (code, exit, hint) = match error.kind() {
            ErrorKind::Authentication => (
                "authentication",
                Exit::Auth,
                "check the API key: set TYPESAFE_API_KEY, or run `jev auth login`",
            ),
            ErrorKind::PermissionDenied => (
                "permission_denied",
                Exit::Auth,
                "this API key is not allowed to do that; use a key with access, or ask TypeSafe to enable it",
            ),
            ErrorKind::BadRequest | ErrorKind::Unprocessable => (
                "api_rejected",
                Exit::ApiRejected,
                "fix the request; `jev validate -f <file>` checks one offline and explains what is wrong",
            ),
            ErrorKind::NotFound => (
                "not_found",
                Exit::ApiRejected,
                "check the model name with `jev models list`, and the base URL",
            ),
            ErrorKind::RateLimit => (
                "rate_limited",
                Exit::RateLimited,
                "wait and try again; for batch work lower --concurrency",
            ),
            ErrorKind::Overloaded => (
                "overloaded",
                Exit::RateLimited,
                "the service is busy; wait and try again",
            ),
            ErrorKind::Timeout => (
                "timeout",
                Exit::Network,
                "try again, or allow more time with --timeout",
            ),
            ErrorKind::Connection => (
                "connection",
                Exit::Network,
                "check the network connection, any proxy settings, and --base-url",
            ),
            ErrorKind::Server | ErrorKind::UnexpectedStatus => (
                "server_error",
                Exit::Network,
                "the service failed; try again later",
            ),
            ErrorKind::InvalidRequest => (
                "usage",
                Exit::Usage,
                "check the flags and values passed to the command",
            ),
            // `ErrorKind` is non-exhaustive, and a response `jev` cannot read is the same problem.
            _ => (
                "invalid_response",
                Exit::Internal,
                "the API answered in a way this version of jev does not understand; run `jev update`",
            ),
        };

        let mut message = error.message().to_owned();
        if error.attempts() > 1 {
            let _ = write!(message, " (after {} attempts)", error.attempts());
        }
        Self {
            code,
            exit,
            message,
            hint: Some(hint.to_owned()),
            error_type: error.error_type().map(str::to_owned),
            request_id: error.request_id().map(str::to_owned),
            http_status: error.status(),
            retryable: error.is_retryable(),
            details: None,
            lines: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use jev_client::{Error, ErrorKind};
    use serde_json::json;

    use super::CliError;
    use crate::exit::Exit;
    use crate::output::Ui;

    #[test]
    fn api_errors_map_to_the_documented_exit_codes() {
        let cases = [
            (ErrorKind::Authentication, 401, Exit::Auth, "authentication"),
            (
                ErrorKind::PermissionDenied,
                403,
                Exit::Auth,
                "permission_denied",
            ),
            (
                ErrorKind::BadRequest,
                400,
                Exit::ApiRejected,
                "api_rejected",
            ),
            (ErrorKind::NotFound, 404, Exit::ApiRejected, "not_found"),
            (
                ErrorKind::Unprocessable,
                422,
                Exit::ApiRejected,
                "api_rejected",
            ),
            (ErrorKind::RateLimit, 429, Exit::RateLimited, "rate_limited"),
            (ErrorKind::Overloaded, 529, Exit::RateLimited, "overloaded"),
            (ErrorKind::Server, 500, Exit::Network, "server_error"),
            (
                ErrorKind::UnexpectedStatus,
                408,
                Exit::Network,
                "server_error",
            ),
        ];

        for (kind, status, exit, code) in cases {
            let error = CliError::from(Error::new(kind, "nope").with_status(status));

            assert_eq!(error.exit, exit, "{kind:?}");
            assert_eq!(error.code, code, "{kind:?}");
            assert_eq!(error.http_status, Some(status));
            assert!(error.hint.is_some(), "{kind:?} names a next action");
        }
    }

    #[test]
    fn failures_without_a_status_map_too() {
        let cases = [
            (ErrorKind::Timeout, Exit::Network, true),
            (ErrorKind::Connection, Exit::Network, true),
            (ErrorKind::InvalidResponse, Exit::Internal, false),
            (ErrorKind::InvalidRequest, Exit::Usage, false),
        ];

        for (kind, exit, retryable) in cases {
            let error = CliError::from(Error::new(kind, "nope"));

            assert_eq!(error.exit, exit, "{kind:?}");
            assert_eq!(error.retryable, retryable, "{kind:?}");
            assert_eq!(error.http_status, None);
        }
    }

    #[test]
    fn the_json_error_always_has_every_documented_field() {
        let source = Error::new(ErrorKind::RateLimit, "Slow down.")
            .with_status(429)
            .with_error_type("rate_limit_error")
            .with_request_id(Some("req_1".to_owned()))
            .with_attempts(3);

        let error = CliError::from(source);

        assert_eq!(
            error.to_json(),
            json!({ "error": {
                "code": "rate_limited",
                "exit_code": 5,
                "error_type": "rate_limit_error",
                "message": "Slow down. (after 3 attempts)",
                "hint": "wait and try again; for batch work lower --concurrency",
                "request_id": "req_1",
                "http_status": 429,
                "retryable": true,
                "details": null
            }})
        );
        assert_eq!(
            CliError::usage("bad flag").to_json(),
            json!({ "error": {
                "code": "usage", "exit_code": 2, "error_type": null, "message": "bad flag",
                "hint": null, "request_id": null, "http_status": null, "retryable": false, "details": null
            }})
        );
    }

    #[test]
    fn the_human_error_is_plain_text_without_colour() {
        let error = CliError::from(
            Error::new(ErrorKind::Authentication, "Invalid API key.")
                .with_request_id(Some("req_9".to_owned())),
        );

        assert_eq!(
            error.to_human(Ui::plain()),
            "error: Invalid API key.\n  hint: check the API key: set TYPESAFE_API_KEY, or run `jev auth login`\n  request id: req_9"
        );
    }

    #[test]
    fn not_implemented_points_at_the_tracking_issue() {
        let error = CliError::not_implemented("batch run", 13);

        assert_eq!(error.exit, Exit::Usage);
        assert_eq!(error.message, "`jev batch run` is not implemented yet");
        assert!(error.hint.unwrap().ends_with("/issues/13"));
    }
}
