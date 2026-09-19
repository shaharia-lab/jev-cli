//! The real transport: HTTPS through `reqwest` and `rustls`, with retries.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, USER_AGENT};
use reqwest::{Method, Url};
use rustls_platform_verifier::BuilderVerifierExt;
use serde::de::DeserializeOwned;
use zeroize::Zeroizing;

use crate::base_url::BaseUrl;
use crate::clock::{Clock, SystemClock};
use crate::error::{Error, ErrorKind};
use crate::models::ModelList;
use crate::request::Request;
use crate::response::Response;
use crate::retry::{RetryPolicy, server_delay};
use crate::secret::ApiKey;
use crate::transport::{Reply, ReplyMeta, Transport};

/// The response header carrying the id of a request.
const REQUEST_ID_HEADER: &str = "x-typesafe-request-id";

/// The largest response body that is read. A real response is a few kilobytes; the cap stops a
/// misbehaving server or proxy from exhausting memory.
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

/// The time allowed to establish a connection, within the per-attempt timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Tracing target for request and response bodies, which are logged only on explicit opt-in.
const BODY_LOG_TARGET: &str = "jev_client::http::body";

/// A [`Transport`] that talks to the TypeSafe API over HTTPS.
///
/// - TLS is `rustls` with the operating system's certificate verifier. Verification cannot be
///   switched off, and a plain `http://` base URL is only possible as described on [`BaseUrl`].
/// - Redirects are not followed, so the API key is never forwarded to another host.
/// - Failures are retried as the [`RetryPolicy`] says, which by default matches the official SDKs.
/// - At `DEBUG` level it traces the method, URL, status, timing, request id and every retry
///   decision. Bodies are traced only after [`HttpTransportBuilder::log_bodies`], because a
///   request's `state` is often customer data. The `Authorization` header is never traced.
///
/// It must be used inside a Tokio runtime.
///
/// ```no_run
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// use jev_client::{ApiKey, HttpTransport, Noul, Request, Transport};
///
/// let key = ApiKey::new(std::env::var("TYPESAFE_API_KEY")?)?;
/// let transport = HttpTransport::builder(key).build()?;
///
/// let request = Request::new("Help! My payouts have been failing for 3 days.", "jev-latest")
///     .question("is_urgent", Noul::new("Does this convey urgency?"));
/// let reply = transport.evaluate(&request).await?;
///
/// println!("{} answered in {:?}", reply.body.model, reply.meta.latency);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct HttpTransport {
    client: reqwest::Client,
    base_url: BaseUrl,
    api_key: ApiKey,
    authorization: HeaderValue,
    retry: RetryPolicy,
    clock: Arc<dyn Clock>,
    log_bodies: bool,
}

impl HttpTransport {
    /// Starts building a transport that authenticates with `api_key`.
    #[must_use]
    pub fn builder(api_key: ApiKey) -> HttpTransportBuilder {
        HttpTransportBuilder {
            api_key,
            base_url: BaseUrl::default(),
            retry: RetryPolicy::default(),
            clock: Arc::new(SystemClock),
            user_agent: None,
            log_bodies: false,
        }
    }

    /// The API root this transport talks to.
    #[must_use]
    pub const fn base_url(&self) -> &BaseUrl {
        &self.base_url
    }

    /// The retry policy in force.
    #[must_use]
    pub const fn retry_policy(&self) -> &RetryPolicy {
        &self.retry
    }

    /// Sends one kind of request until it succeeds, fails for good, or runs out of retries.
    async fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<Reply<T>, Error> {
        let url = self.base_url.endpoint(path).map_err(|error| {
            Error::new(ErrorKind::InvalidRequest, error.to_string()).with_source(error)
        })?;
        if self.base_url.sends_key_in_clear() {
            tracing::warn!(%url, "sending the API key over unencrypted HTTP");
        }

        let started = self.clock.now();
        let mut retry = 0;
        loop {
            let attempt = retry + 1;
            let outcome = self
                .attempt::<T>(&method, &url, body.as_deref(), attempt)
                .await;

            let failure = match outcome {
                Ok((parsed, request_id)) => {
                    let latency = self.clock.now().duration_since(started);
                    return Ok(Reply::new(
                        parsed,
                        ReplyMeta::new(request_id, latency, attempt),
                    ));
                }
                Err(failure) => failure,
            };

            if retry >= self.retry.max_retries || !self.should_retry(&failure) {
                if failure.is_retryable() {
                    tracing::debug!(attempt, "giving up: no retries left");
                }
                return Err(failure.with_attempts(attempt));
            }

            let delay = self
                .retry
                .delay(retry, failure.retry_after(), fastrand::f64());
            tracing::debug!(
                attempt,
                max_retries = self.retry.max_retries,
                delay_ms = delay.as_millis(),
                server_requested = failure.retry_after().is_some(),
                reason = %failure_label(&failure),
                "retrying"
            );
            self.clock.sleep(delay).await;
            retry += 1;
        }
    }

    /// Whether the policy retries this failure.
    fn should_retry(&self, failure: &Error) -> bool {
        match (failure.kind(), failure.status()) {
            (ErrorKind::Timeout, _) => self.retry.retry_timeouts,
            (ErrorKind::Connection, _) => self.retry.retry_connection_errors,
            (_, Some(status)) => self.retry.retries_status(status),
            _ => false,
        }
    }

    /// One attempt: send, read, and either parse the body or describe the failure.
    async fn attempt<T: DeserializeOwned>(
        &self,
        method: &Method,
        url: &Url,
        body: Option<&[u8]>,
        attempt: u32,
    ) -> Result<(T, Option<String>), Error> {
        tracing::debug!(%method, %url, attempt, "sending request");
        if let (true, Some(body)) = (self.log_bodies, body) {
            let body = self.api_key.scrub(&String::from_utf8_lossy(body));
            tracing::debug!(target: BODY_LOG_TARGET, %body, "request body");
        }

        let mut builder = self
            .client
            .request(method.clone(), url.clone())
            .header(AUTHORIZATION, self.authorization.clone())
            .header(ACCEPT, HeaderValue::from_static("application/json"));
        if let Some(body) = body {
            builder = builder
                .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                .body(body.to_vec());
        }

        let sent = self.clock.now();
        let response = builder
            .send()
            .await
            .map_err(|error| self.transport_error(error))?;

        let status = response.status();
        let request_id = request_id(response.headers());
        let retry_after = server_delay(response.headers(), self.clock.wall_time());
        let bytes = read_capped(response).await.map_err(|error| match error {
            ReadError::Transport(error) => self
                .transport_error(error)
                .with_request_id(request_id.clone()),
            ReadError::TooLarge => Error::new(
                ErrorKind::InvalidResponse,
                format!("the response is larger than {MAX_RESPONSE_BYTES} bytes"),
            )
            .with_status(status.as_u16())
            .with_request_id(request_id.clone()),
        })?;

        tracing::debug!(
            %method,
            %url,
            status = status.as_u16(),
            elapsed_ms = self.clock.now().duration_since(sent).as_millis(),
            request_id = request_id.as_deref().unwrap_or("-"),
            attempt,
            "received response"
        );
        if self.log_bodies {
            // Opting in to bodies is opting in to seeing `state`, never the key: a server that
            // echoes a header back must not get it into a log.
            let body = self.api_key.scrub(&String::from_utf8_lossy(&bytes));
            tracing::debug!(target: BODY_LOG_TARGET, %body, "response body");
        }

        if !status.is_success() {
            return Err(Error::from_response(status.as_u16(), &bytes, |text| {
                self.api_key.scrub(text)
            })
            .with_request_id(request_id)
            .with_retry_after(retry_after));
        }
        match serde_json::from_slice::<T>(&bytes) {
            Ok(parsed) => Ok((parsed, request_id)),
            Err(error) => Err(
                Error::new(ErrorKind::InvalidResponse, describe_json_error(&error))
                    .with_status(status.as_u16())
                    .with_request_id(request_id),
            ),
        }
    }

    /// Describes a failure to send a request or to read its response.
    fn transport_error(&self, error: reqwest::Error) -> Error {
        let kind = if error.is_timeout() {
            ErrorKind::Timeout
        } else if error.is_builder() {
            ErrorKind::InvalidRequest
        } else {
            ErrorKind::Connection
        };
        let message = match kind {
            ErrorKind::Timeout => format!("no response within {:?}", self.retry.timeout),
            _ => self.api_key.scrub(&root_cause(&error)),
        };
        // The URL is dropped from the source as a precaution; the trace output already has it.
        Error::new(kind, message).with_source(error.without_url())
    }
}

impl Transport for HttpTransport {
    async fn evaluate(&self, request: &Request) -> Result<Reply<Response>, Error> {
        let body = serde_json::to_vec(request).map_err(|error| {
            Error::new(
                ErrorKind::InvalidRequest,
                format!("the request could not be serialised: {error}"),
            )
            .with_source(error)
        })?;
        self.call(Method::POST, "v1/systemone", Some(body)).await
    }

    async fn list_models(&self) -> Result<Reply<ModelList>, Error> {
        self.call(Method::GET, "v1/models", None).await
    }
}

impl fmt::Debug for HttpTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpTransport")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field("retry", &self.retry)
            .field("log_bodies", &self.log_bodies)
            .finish_non_exhaustive()
    }
}

/// Configures and builds an [`HttpTransport`].
pub struct HttpTransportBuilder {
    api_key: ApiKey,
    base_url: BaseUrl,
    retry: RetryPolicy,
    clock: Arc<dyn Clock>,
    user_agent: Option<String>,
    log_bodies: bool,
}

impl HttpTransportBuilder {
    /// Sets the API root. The default is `https://api.typesafe.ai`.
    #[must_use]
    pub fn base_url(mut self, base_url: BaseUrl) -> Self {
        self.base_url = base_url;
        self
    }

    /// Sets the retry policy, which also holds the per-attempt timeout.
    #[must_use]
    pub fn retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// Names the application for the `User-Agent` header, as `name/version`. It is placed in front
    /// of this crate's own `jev-client/<version>`.
    #[must_use]
    pub fn user_agent(mut self, product: impl Into<String>) -> Self {
        self.user_agent = Some(product.into());
        self
    }

    /// Traces request and response bodies at `DEBUG` level, under the target
    /// `jev_client::http::body`.
    ///
    /// Off by default, and meant to stay off: a request's `state` is often customer data, and an
    /// error response can echo it back. The `Authorization` header is not traced either way.
    #[must_use]
    pub const fn log_bodies(mut self, enabled: bool) -> Self {
        self.log_bodies = enabled;
        self
    }

    /// Replaces the clock. Tests use this to observe retry delays without waiting for them.
    #[must_use]
    pub fn clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Builds the transport.
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorKind::InvalidRequest`] when the user agent is not a valid header value,
    /// or when the TLS stack cannot be initialised on this system.
    pub fn build(self) -> Result<HttpTransport, Error> {
        let invalid = |what: &str, error: &dyn fmt::Display| {
            Error::new(ErrorKind::InvalidRequest, format!("{what}: {error}"))
        };

        let own_agent = concat!("jev-client/", env!("CARGO_PKG_VERSION"));
        let user_agent = self.user_agent.map_or_else(
            || own_agent.to_owned(),
            |product| format!("{product} {own_agent}"),
        );
        let user_agent = HeaderValue::from_str(&user_agent)
            .map_err(|error| invalid("invalid user agent", &error))?;

        let bearer = Zeroizing::new(format!("Bearer {}", self.api_key.expose()));
        // `ApiKey` only admits visible ASCII, so this cannot fail; the error never holds the key.
        let mut authorization = HeaderValue::from_str(&bearer).map_err(|_| {
            Error::new(
                ErrorKind::InvalidRequest,
                "the API key cannot be sent in a header",
            )
        })?;
        authorization.set_sensitive(true);

        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let tls = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|error| invalid("TLS could not be initialised", &error))?
            .with_platform_verifier()
            .map_err(|error| {
                invalid(
                    "the system certificate verifier could not be initialised",
                    &error,
                )
            })?
            .with_no_client_auth();

        let client = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .https_only(self.base_url.is_https())
            .redirect(reqwest::redirect::Policy::none())
            .default_headers(HeaderMap::from_iter([(USER_AGENT, user_agent)]))
            .timeout(self.retry.timeout)
            .connect_timeout(CONNECT_TIMEOUT.min(self.retry.timeout))
            .build()
            .map_err(|error| invalid("the HTTP client could not be built", &error))?;

        Ok(HttpTransport {
            client,
            base_url: self.base_url,
            api_key: self.api_key,
            authorization,
            retry: self.retry,
            clock: self.clock,
            log_bodies: self.log_bodies,
        })
    }
}

impl fmt::Debug for HttpTransportBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpTransportBuilder")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field("retry", &self.retry)
            .field("user_agent", &self.user_agent)
            .field("log_bodies", &self.log_bodies)
            .finish_non_exhaustive()
    }
}

/// A short label for a failure in trace output: the status when there is one, else the kind.
fn failure_label(failure: &Error) -> String {
    failure.status().map_or_else(
        || format!("{:?}", failure.kind()),
        |status| format!("HTTP {status}"),
    )
}

fn request_id(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(REQUEST_ID_HEADER)?.to_str().ok()?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

enum ReadError {
    Transport(reqwest::Error),
    TooLarge,
}

/// Reads a response body, refusing to buffer more than [`MAX_RESPONSE_BYTES`].
async fn read_capped(mut response: reqwest::Response) -> Result<Vec<u8>, ReadError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(ReadError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(ReadError::Transport)? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(ReadError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Describes why a body failed to parse, by category and position only.
///
/// `serde_json`'s own message is not used, and the error is not kept as a source, because a type
/// mismatch quotes the offending value (`invalid type: string "..."`) and a response can carry
/// text that came from the request. Body logging is the way to see the content.
fn describe_json_error(error: &serde_json::Error) -> String {
    use serde_json::error::Category;

    let problem = match error.classify() {
        Category::Syntax => "the body is not valid JSON",
        Category::Eof => "the body ends unexpectedly",
        Category::Data => "the body is JSON but not in the documented shape",
        Category::Io => "the body could not be read",
    };
    format!(
        "{problem} (line {}, column {})",
        error.line(),
        error.column()
    )
}

/// The innermost cause of an error, which is where `reqwest` keeps the useful part
/// ("connection refused", "certificate has expired", ...).
fn root_cause(error: &(dyn std::error::Error + 'static)) -> String {
    let mut cause = error;
    while let Some(next) = cause.source() {
        cause = next;
    }
    cause.to_string()
}
