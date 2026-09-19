//! The error a transport returns, and how an API error body becomes one.

use std::error::Error as StdError;
use std::fmt::{self, Write as _};
use std::time::Duration;

use serde_json::Value;

/// The longest server-supplied message that is kept. Longer ones are cut, with an ellipsis.
const MAX_MESSAGE_CHARS: usize = 600;

/// How many entries of a validation error list are spelt out before the rest are counted.
const MAX_VALIDATION_ENTRIES: usize = 5;

/// What went wrong, as a category a caller can act on.
///
/// The HTTP kinds mirror the error classes of the official SDKs. A command-line tool maps these
/// to exit codes; a program decides whether to give up, re-authenticate or slow down.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// 400: the API could not make sense of the request.
    BadRequest,
    /// 401: the API key is missing or not valid.
    Authentication,
    /// 403: the key is valid but may not do this.
    PermissionDenied,
    /// 404: no such endpoint or model.
    NotFound,
    /// 422: the request body failed the API's validation.
    Unprocessable,
    /// 429: too many requests or tokens. Slow down.
    RateLimit,
    /// 529: the service is temporarily overloaded.
    Overloaded,
    /// Any other 5xx: the service failed.
    Server,
    /// A status this crate has no better category for, such as 408 or an unexpected redirect.
    UnexpectedStatus,
    /// An attempt did not complete within the timeout.
    Timeout,
    /// The server could not be reached, or the connection was lost before the response was read.
    Connection,
    /// The server answered with success, but the body was not what the API documents.
    InvalidResponse,
    /// The request could not be built or sent for a reason that is the caller's to fix.
    InvalidRequest,
}

impl ErrorKind {
    /// Returns `true` when trying again later can reasonably succeed without changing anything.
    #[must_use]
    pub const fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimit | Self::Overloaded | Self::Server | Self::Timeout | Self::Connection
        )
    }

    /// The category of an HTTP status that is not a success.
    pub(crate) const fn from_status(status: u16) -> Self {
        match status {
            400 => Self::BadRequest,
            401 => Self::Authentication,
            403 => Self::PermissionDenied,
            404 => Self::NotFound,
            422 => Self::Unprocessable,
            429 => Self::RateLimit,
            529 => Self::Overloaded,
            500..=599 => Self::Server,
            _ => Self::UnexpectedStatus,
        }
    }

    const fn describe(self) -> &'static str {
        match self {
            Self::BadRequest => "the API rejected the request",
            Self::Authentication => "authentication failed",
            Self::PermissionDenied => "permission denied",
            Self::NotFound => "not found",
            Self::Unprocessable => "the request failed the API's validation",
            Self::RateLimit => "rate limited",
            Self::Overloaded => "the API is overloaded",
            Self::Server => "the API failed",
            Self::UnexpectedStatus => "unexpected response from the API",
            Self::Timeout => "the request timed out",
            Self::Connection => "could not reach the API",
            Self::InvalidResponse => "the API returned a response that could not be read",
            Self::InvalidRequest => "the request could not be sent",
        }
    }
}

/// An error from a [`Transport`](crate::Transport).
///
/// Every error carries its [`ErrorKind`] and a message, plus whatever the server supplied: the
/// HTTP status, the API's own `error_type`, and the `x-typesafe-request-id` to quote when asking
/// TypeSafe for help. Nothing in an error ever contains the API key or a request's `state`.
///
/// ```no_run
/// # async fn example(transport: impl jev_client::Transport, request: jev_client::Request) {
/// use jev_client::ErrorKind;
///
/// if let Err(error) = transport.evaluate(&request).await {
///     match error.kind() {
///         ErrorKind::Authentication => eprintln!("check your API key"),
///         kind if kind.is_retryable() => eprintln!("try again later: {error}"),
///         _ => eprintln!("{error}"),
///     }
/// }
/// # }
/// ```
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    status: Option<u16>,
    api_type: Option<String>,
    request_id: Option<String>,
    retry_after: Option<Duration>,
    attempts: u32,
    source: Option<Box<dyn StdError + Send + Sync>>,
}

impl Error {
    /// An error of the given kind.
    ///
    /// [`HttpTransport`](crate::HttpTransport) builds its own errors; this constructor and the
    /// `with_*` methods exist so that a fake [`Transport`](crate::Transport) in a test can return
    /// the same errors the real one would.
    ///
    /// ```
    /// use jev_client::{Error, ErrorKind};
    ///
    /// let error = Error::new(ErrorKind::RateLimit, "Slow down.")
    ///     .with_status(429)
    ///     .with_error_type("rate_limit_error")
    ///     .with_request_id(Some("req_123".to_owned()));
    ///
    /// assert!(error.is_retryable());
    /// assert_eq!(error.request_id(), Some("req_123"));
    /// ```
    #[must_use]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            status: None,
            api_type: None,
            request_id: None,
            retry_after: None,
            attempts: 0,
            source: None,
        }
    }

    /// An error for a response whose status is not a success.
    ///
    /// `body` is the raw response body and `scrub` removes secrets from text the server supplied.
    pub(crate) fn from_response(status: u16, body: &[u8], scrub: impl Fn(&str) -> String) -> Self {
        let detail = ApiDetail::parse(body);
        let message = detail.message.map_or_else(
            || format!("HTTP {status} with no error message"),
            |message| scrub(&truncate(&message)),
        );

        let mut error = Self::new(ErrorKind::from_status(status), message);
        error.status = Some(status);
        error.api_type = detail
            .error_type
            .map(|error_type| scrub(&truncate(&error_type)));
        error
    }

    pub(crate) fn with_source(mut self, source: impl StdError + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// Sets the HTTP status.
    #[must_use]
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    /// Sets the API's machine-readable error type, such as `rate_limit_error`.
    #[must_use]
    pub fn with_error_type(mut self, error_type: impl Into<String>) -> Self {
        self.api_type = Some(error_type.into());
        self
    }

    /// Sets the `x-typesafe-request-id`.
    #[must_use]
    pub fn with_request_id(mut self, request_id: Option<String>) -> Self {
        self.request_id = request_id;
        self
    }

    /// Sets the delay the server asked for.
    #[must_use]
    pub const fn with_retry_after(mut self, retry_after: Option<Duration>) -> Self {
        self.retry_after = retry_after;
        self
    }

    /// Sets how many attempts were made.
    #[must_use]
    pub const fn with_attempts(mut self, attempts: u32) -> Self {
        self.attempts = attempts;
        self
    }

    /// The category of the error.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The message: the API's own, when it sent one.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The HTTP status, when the server answered.
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        self.status
    }

    /// The API's machine-readable error type, such as `authentication_error`, when it sent one.
    #[must_use]
    pub fn error_type(&self) -> Option<&str> {
        self.api_type.as_deref()
    }

    /// The `x-typesafe-request-id` of the failed attempt, when the server sent one.
    #[must_use]
    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }

    /// The delay the server asked for before trying again, when it sent one.
    #[must_use]
    pub const fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }

    /// How many attempts were made, including the first. `0` when no request was sent.
    #[must_use]
    pub const fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Returns `true` when trying again later can reasonably succeed. See
    /// [`ErrorKind::is_retryable`]; a 408 also counts.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        self.kind.is_retryable() || self.status == Some(408)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.describe())?;
        match (self.status, &self.api_type) {
            (Some(status), Some(api_type)) => write!(formatter, " (HTTP {status}, {api_type})")?,
            (Some(status), None) => write!(formatter, " (HTTP {status})")?,
            (None, _) => {}
        }
        write!(formatter, ": {}", self.message)?;
        if self.attempts > 1 {
            write!(formatter, " (after {} attempts)", self.attempts)?;
        }
        if let Some(request_id) = &self.request_id {
            write!(formatter, " [request id: {request_id}]")?;
        }
        Ok(())
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn StdError + 'static))
    }
}

/// The useful parts of an API error body.
#[derive(Debug, Default, PartialEq, Eq)]
struct ApiDetail {
    error_type: Option<String>,
    message: Option<String>,
}

impl ApiDetail {
    /// Reads an error body. The API uses three shapes, all under a top-level `detail`:
    ///
    /// - `{"error_type": "...", "message": "..."}`, the documented one;
    /// - a bare string;
    /// - for 422, a list of `{"type", "loc", "msg", "input"}` entries.
    ///
    /// The list's `input` echoes the request, `state` included, so it is never read: only `loc`
    /// and `msg` make it into the message. A body in any other shape yields nothing, rather than
    /// being quoted, for the same reason.
    fn parse(body: &[u8]) -> Self {
        let Ok(Value::Object(root)) = serde_json::from_slice::<Value>(body) else {
            return Self::default();
        };
        match root.get("detail") {
            Some(Value::String(message)) => Self {
                error_type: None,
                message: Some(message.clone()),
            },
            Some(Value::Object(detail)) => Self {
                error_type: detail
                    .get("error_type")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                message: detail
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            },
            Some(Value::Array(entries)) => Self {
                error_type: None,
                message: validation_message(entries),
            },
            _ => Self::default(),
        }
    }
}

/// One line per distinct location in a validation error list, such as
/// `questions.frustration.criteria.2: Input should be a valid string`.
fn validation_message(entries: &[Value]) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    for entry in entries {
        let Some(message) = entry.get("msg").and_then(Value::as_str) else {
            continue;
        };
        let location = entry
            .get("loc")
            .and_then(Value::as_array)
            .map(|loc| location(loc));
        let line = match location.as_deref() {
            Some(location) if !location.is_empty() => format!("{location}: {message}"),
            _ => message.to_owned(),
        };
        // A union-typed field reports once per alternative; the location is what matters.
        let prefix = line.split(": ").next().unwrap_or_default().to_owned();
        if !lines
            .iter()
            .any(|seen| seen.split(": ").next() == Some(prefix.as_str()))
        {
            lines.push(line);
        }
    }
    if lines.is_empty() {
        return None;
    }

    let more = lines.len().saturating_sub(MAX_VALIDATION_ENTRIES);
    lines.truncate(MAX_VALIDATION_ENTRIES);
    let mut message = lines.join("; ");
    if more > 0 {
        let _ = write!(message, "; and {more} more");
    }
    Some(message)
}

/// A dotted path from a `loc` array, without the leading `body` and without the trailing name of
/// the type alternative that was tried (`str`, `dict[any,any]`, `list[any]`).
fn location(loc: &[Value]) -> String {
    let mut parts: Vec<String> = loc
        .iter()
        .filter_map(|part| match part {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
        .collect();
    if parts.first().is_some_and(|part| part == "body") {
        parts.remove(0);
    }
    if parts
        .last()
        .is_some_and(|part| matches!(part.as_str(), "str" | "dict[any,any]" | "list[any]"))
    {
        parts.pop();
    }
    parts.join(".")
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_MESSAGE_CHARS {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(MAX_MESSAGE_CHARS).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::{ApiDetail, Error, ErrorKind};

    fn keep(text: &str) -> String {
        text.to_owned()
    }

    #[test]
    fn statuses_map_to_the_sdk_error_classes() {
        let cases = [
            (400, ErrorKind::BadRequest),
            (401, ErrorKind::Authentication),
            (403, ErrorKind::PermissionDenied),
            (404, ErrorKind::NotFound),
            (422, ErrorKind::Unprocessable),
            (429, ErrorKind::RateLimit),
            (500, ErrorKind::Server),
            (503, ErrorKind::Server),
            (529, ErrorKind::Overloaded),
            (302, ErrorKind::UnexpectedStatus),
            (408, ErrorKind::UnexpectedStatus),
            (418, ErrorKind::UnexpectedStatus),
        ];

        for (status, kind) in cases {
            assert_eq!(ErrorKind::from_status(status), kind, "{status}");
        }
    }

    #[test]
    fn only_transient_failures_are_retryable() {
        let retryable = [
            ErrorKind::RateLimit,
            ErrorKind::Overloaded,
            ErrorKind::Server,
            ErrorKind::Timeout,
            ErrorKind::Connection,
        ];
        let permanent = [
            ErrorKind::BadRequest,
            ErrorKind::Authentication,
            ErrorKind::PermissionDenied,
            ErrorKind::NotFound,
            ErrorKind::Unprocessable,
            ErrorKind::UnexpectedStatus,
            ErrorKind::InvalidResponse,
            ErrorKind::InvalidRequest,
        ];

        assert!(retryable.iter().all(|kind| kind.is_retryable()));
        assert!(permanent.iter().all(|kind| !kind.is_retryable()));
        assert!(
            Error::from_response(408, b"", keep).is_retryable(),
            "408 is retryable by status"
        );
    }

    #[test]
    fn the_documented_error_body_keeps_the_error_type_and_message() {
        let body =
            br#"{"detail":{"error_type":"authentication_error","message":"Invalid API key."}}"#;

        let error = Error::from_response(401, body, keep)
            .with_request_id(Some("req_1".into()))
            .with_attempts(1);

        assert_eq!(error.kind(), ErrorKind::Authentication);
        assert_eq!(error.status(), Some(401));
        assert_eq!(error.error_type(), Some("authentication_error"));
        assert_eq!(error.message(), "Invalid API key.");
        assert_eq!(
            error.to_string(),
            "authentication failed (HTTP 401, authentication_error): Invalid API key. [request id: req_1]"
        );
    }

    #[test]
    fn a_bare_string_detail_is_the_message() {
        let body = br#"{"detail":"Noul question must have criteria or instructions: q"}"#;

        let error = Error::from_response(400, body, keep);

        assert_eq!(error.error_type(), None);
        assert_eq!(
            error.message(),
            "Noul question must have criteria or instructions: q"
        );
    }

    #[test]
    fn a_validation_list_names_each_location_once_and_never_echoes_the_input() {
        let body = br#"{"detail":[
            {"type":"string_type","loc":["body","questions","frustration","score","criteria",2,"str"],"msg":"Input should be a valid string","input":"SECRET STATE"},
            {"type":"dict_type","loc":["body","questions","frustration","score","criteria",2,"dict[any,any]"],"msg":"Input should be a valid dictionary","input":"SECRET STATE"},
            {"type":"missing","loc":["body","state"],"msg":"Field required","input":{"state":"SECRET STATE"}}
        ]}"#;

        let error = Error::from_response(422, body, keep);

        assert_eq!(error.kind(), ErrorKind::Unprocessable);
        assert_eq!(
            error.message(),
            "questions.frustration.score.criteria.2: Input should be a valid string; state: Field required"
        );
        assert!(!format!("{error} {error:?}").contains("SECRET STATE"));
    }

    #[test]
    fn a_long_validation_list_is_summarised() {
        let entries: Vec<String> = (0..9)
            .map(|index| format!(r#"{{"loc":["body","questions","q{index}"],"msg":"bad"}}"#))
            .collect();
        let body = format!(r#"{{"detail":[{}]}}"#, entries.join(","));

        let error = Error::from_response(422, body.as_bytes(), keep);

        assert!(
            error
                .message()
                .starts_with("questions.q0: bad; questions.q1: bad;"),
            "{}",
            error.message()
        );
        assert!(
            error.message().ends_with("questions.q4: bad; and 4 more"),
            "{}",
            error.message()
        );
    }

    #[test]
    fn a_body_in_an_unknown_shape_is_never_quoted() {
        for body in [
            &b"<html>SECRET STATE</html>"[..],
            br#"{"error":"SECRET STATE"}"#,
            br#"["SECRET STATE"]"#,
            br#"{"detail":42}"#,
            b"",
        ] {
            let error = Error::from_response(502, body, keep);

            assert_eq!(error.message(), "HTTP 502 with no error message");
            assert!(!format!("{error:?}").contains("SECRET STATE"));
        }
        assert_eq!(
            ApiDetail::parse(br#"{"detail":{"message":7}}"#),
            ApiDetail::default()
        );
    }

    #[test]
    fn server_text_is_scrubbed_and_cut_to_a_sane_length() {
        let body = format!(
            r#"{{"detail":{{"error_type":"echo sentinel","message":"bad token sentinel {}"}}}}"#,
            "x".repeat(2000)
        );

        let error = Error::from_response(401, body.as_bytes(), |text| {
            text.replace("sentinel", "[REDACTED]")
        });

        assert!(!error.to_string().contains("sentinel"), "{error}");
        assert_eq!(error.error_type(), Some("echo [REDACTED]"));
        assert!(error.message().chars().count() < 700);
        assert!(error.message().ends_with('…'));
    }

    #[test]
    fn display_mentions_attempts_only_when_there_were_retries_and_keeps_the_source() {
        let cause = std::io::Error::other("connection reset");
        let error = Error::new(ErrorKind::Connection, "connection reset")
            .with_source(cause)
            .with_attempts(3);

        assert_eq!(
            error.to_string(),
            "could not reach the API: connection reset (after 3 attempts)"
        );
        assert_eq!(
            error.source().map(ToString::to_string),
            Some("connection reset".to_owned())
        );
        assert_eq!(error.attempts(), 3);
    }
}
