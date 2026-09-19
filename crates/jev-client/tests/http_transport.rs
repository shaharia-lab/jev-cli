//! `HttpTransport` against a local mock server: retries, error mapping, request ids, and the
//! guarantee that the API key never shows up anywhere it could be read.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use jev_client::{
    Answer, ApiKey, BaseUrl, Clock, ErrorKind, HttpTransport, HttpTransportBuilder, Noul, Request,
    RetryPolicy, Transport,
};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A key that must never appear in any output. It is not a real credential.
const SENTINEL_KEY: &str = "sentinel-key-do-not-leak-7f3a9c";

/// A state that must never appear in logs or errors unless body logging is switched on.
const SENTINEL_STATE: &str = "sentinel-state-customer-data-51b2";

/// A clock that returns from `sleep` at once, recording the delay and advancing its own time.
#[derive(Default)]
struct FakeClock {
    sleeps: Mutex<Vec<Duration>>,
    elapsed: Mutex<Duration>,
    origin: Option<Instant>,
}

impl FakeClock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            origin: Some(Instant::now()),
            ..Self::default()
        })
    }

    fn sleeps(&self) -> Vec<Duration> {
        self.sleeps.lock().unwrap().clone()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.origin.unwrap() + *self.elapsed.lock().unwrap()
    }

    fn wall_time(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000) // 2001-09-09T01:46:40Z
    }

    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.sleeps.lock().unwrap().push(duration);
        *self.elapsed.lock().unwrap() += duration;
        Box::pin(async {})
    }
}

fn builder(server: &MockServer, clock: &Arc<FakeClock>) -> HttpTransportBuilder {
    HttpTransport::builder(ApiKey::new(SENTINEL_KEY.to_owned()).unwrap())
        .base_url(BaseUrl::parse(&server.uri()).unwrap())
        .clock(Arc::clone(clock) as Arc<dyn Clock>)
}

fn transport(server: &MockServer, clock: &Arc<FakeClock>) -> HttpTransport {
    builder(server, clock).build().unwrap()
}

fn request() -> Request {
    Request::new(SENTINEL_STATE, "jev-latest")
        .question("is_urgent", Noul::new("Does this convey urgency?"))
}

fn success() -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("x-typesafe-request-id", "req_ok")
        .set_body_json(json!({
            "model": "jev-1.13.0",
            "answers": { "is_urgent": { "type": "noul", "noul": 0.92 } },
            "usage": { "input_tokens": 312, "output_tokens": 48 }
        }))
}

fn api_error(status: u16, error_type: &str, message: &str) -> ResponseTemplate {
    ResponseTemplate::new(status)
        .insert_header("x-typesafe-request-id", format!("req_{status}"))
        .set_body_json(json!({ "detail": { "error_type": error_type, "message": message } }))
}

/// Mounts `response` for the next `times` evaluation calls, then lets later mocks take over.
async fn respond(server: &MockServer, response: ResponseTemplate, times: u64) {
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(response)
        .up_to_n_times(times)
        .expect(times)
        .mount(server)
        .await;
}

async fn requests_received(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().len()
}

#[tokio::test]
async fn a_successful_evaluation_sends_the_request_as_written_and_returns_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(header(
            "authorization",
            format!("Bearer {SENTINEL_KEY}").as_str(),
        ))
        .and(header("content-type", "application/json"))
        .and(header("accept", "application/json"))
        .respond_with(success())
        .expect(1)
        .mount(&server)
        .await;
    let clock = FakeClock::new();
    let transport = builder(&server, &clock)
        .user_agent("jev/9.9.9")
        .build()
        .unwrap();

    let reply = transport.evaluate(&request()).await.unwrap();

    assert_eq!(reply.body.model, "jev-1.13.0");
    assert_eq!(reply.body.usage.input_tokens, 312);
    assert_eq!(
        reply
            .body
            .answers
            .get("is_urgent")
            .and_then(Answer::as_noul)
            .map(|answer| answer.noul),
        Some(0.92)
    );
    assert_eq!(reply.meta.request_id.as_deref(), Some("req_ok"));
    assert_eq!(reply.meta.attempts, 1);
    assert!(clock.sleeps().is_empty());
    let raw: Value = serde_json::from_str(reply.raw_body.as_deref().unwrap()).unwrap();
    assert_eq!(
        raw["answers"]["is_urgent"]["noul"], 0.92,
        "the body is kept as received"
    );

    let received = &server.received_requests().await.unwrap()[0];
    let sent: Value = serde_json::from_slice(&received.body).unwrap();
    assert_eq!(sent, serde_json::to_value(request()).unwrap());
    let agent = received
        .headers
        .get("user-agent")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(
        agent,
        format!("jev/9.9.9 jev-client/{}", jev_client::VERSION)
    );
}

#[tokio::test]
async fn a_429_with_retry_after_waits_as_asked_and_then_succeeds() {
    let server = MockServer::start().await;
    respond(
        &server,
        api_error(429, "rate_limit_error", "Slow down.").insert_header("retry-after", "1"),
        1,
    )
    .await;
    respond(&server, success(), 1).await;
    let clock = FakeClock::new();

    let reply = transport(&server, &clock)
        .evaluate(&request())
        .await
        .unwrap();

    assert_eq!(
        clock.sleeps(),
        [Duration::from_secs(1)],
        "the server's delay, not backoff"
    );
    assert_eq!(reply.meta.attempts, 2);
    assert_eq!(reply.meta.request_id.as_deref(), Some("req_ok"));
    assert!(
        reply.meta.latency >= Duration::from_secs(1),
        "latency includes the wait"
    );
}

#[tokio::test]
async fn a_529_is_retried_with_jittered_backoff_and_then_succeeds() {
    let server = MockServer::start().await;
    respond(
        &server,
        api_error(529, "overloaded_error", "Overloaded."),
        1,
    )
    .await;
    respond(&server, success(), 1).await;
    let clock = FakeClock::new();

    let reply = transport(&server, &clock)
        .evaluate(&request())
        .await
        .unwrap();

    let sleeps = clock.sleeps();
    assert_eq!(sleeps.len(), 1);
    assert!(
        (Duration::from_millis(375)..=Duration::from_millis(500)).contains(&sleeps[0]),
        "first backoff is 0.5 s minus up to a quarter, got {:?}",
        sleeps[0]
    );
    assert_eq!(reply.meta.attempts, 2);
}

#[tokio::test]
async fn three_500s_fail_as_a_server_error_after_exactly_two_retries() {
    let server = MockServer::start().await;
    respond(&server, api_error(500, "internal_error", "Boom."), 3).await;
    let clock = FakeClock::new();

    let error = transport(&server, &clock)
        .evaluate(&request())
        .await
        .unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Server);
    assert_eq!(error.status(), Some(500));
    assert_eq!(error.attempts(), 3);
    assert!(error.is_retryable());
    assert_eq!(
        requests_received(&server).await,
        3,
        "one attempt and exactly two retries"
    );
    let sleeps = clock.sleeps();
    assert_eq!(sleeps.len(), 2);
    assert!(
        sleeps[1] > sleeps[0] || sleeps[1] >= Duration::from_millis(750),
        "backoff grows: {sleeps:?}"
    );
    assert!(error.to_string().contains("after 3 attempts"), "{error}");
}

#[tokio::test]
async fn a_401_and_a_422_are_not_retried_and_keep_the_api_error_type() {
    let cases = [
        (401, "authentication_error", ErrorKind::Authentication),
        (422, "invalid_request_error", ErrorKind::Unprocessable),
        (400, "api_usage_error", ErrorKind::BadRequest),
        (403, "permission_error", ErrorKind::PermissionDenied),
        (404, "not_found_error", ErrorKind::NotFound),
    ];
    for (status, error_type, kind) in cases {
        let server = MockServer::start().await;
        respond(&server, api_error(status, error_type, "Nope."), 1).await;
        let clock = FakeClock::new();

        let error = transport(&server, &clock)
            .evaluate(&request())
            .await
            .unwrap_err();

        assert_eq!(error.kind(), kind, "{status}");
        assert_eq!(error.status(), Some(status));
        assert_eq!(error.error_type(), Some(error_type));
        assert_eq!(error.message(), "Nope.");
        assert_eq!(
            error.request_id(),
            Some(format!("req_{status}").as_str()),
            "request id on the error"
        );
        assert_eq!(error.attempts(), 1);
        assert!(!error.is_retryable());
        assert_eq!(
            requests_received(&server).await,
            1,
            "{status} must not be retried"
        );
        assert!(clock.sleeps().is_empty());
    }
}

#[tokio::test]
async fn a_422_validation_list_is_summarised_without_echoing_the_state() {
    let server = MockServer::start().await;
    let body = json!({ "detail": [{
        "type": "string_type",
        "loc": ["body", "questions", "frustration", "score", "criteria", 2, "str"],
        "msg": "Input should be a valid string",
        "input": SENTINEL_STATE
    }]});
    respond(&server, ResponseTemplate::new(422).set_body_json(body), 1).await;

    let error = transport(&server, &FakeClock::new())
        .evaluate(&request())
        .await
        .unwrap_err();

    assert_eq!(
        error.message(),
        "questions.frustration.score.criteria.2: Input should be a valid string"
    );
    assert!(!format!("{error} {error:?}").contains(SENTINEL_STATE));
}

#[tokio::test]
async fn retrying_can_be_switched_off() {
    let server = MockServer::start().await;
    respond(&server, api_error(503, "unavailable", "Down."), 1).await;
    let clock = FakeClock::new();
    let transport = builder(&server, &clock)
        .retry_policy(RetryPolicy::none())
        .build()
        .unwrap();

    let error = transport.evaluate(&request()).await.unwrap_err();

    assert_eq!(error.attempts(), 1);
    assert_eq!(requests_received(&server).await, 1);
    assert!(clock.sleeps().is_empty());
}

#[tokio::test]
async fn an_excessive_retry_after_falls_back_to_backoff_and_is_reported_on_the_error() {
    let server = MockServer::start().await;
    respond(
        &server,
        api_error(429, "rate_limit_error", "Slow down.").insert_header("retry-after", "3600"),
        3,
    )
    .await;
    let clock = FakeClock::new();

    let error = transport(&server, &clock)
        .evaluate(&request())
        .await
        .unwrap_err();

    assert_eq!(error.kind(), ErrorKind::RateLimit);
    assert_eq!(error.retry_after(), Some(Duration::from_secs(3600)));
    assert!(
        clock
            .sleeps()
            .iter()
            .all(|sleep| *sleep <= Duration::from_secs(5)),
        "{:?}",
        clock.sleeps()
    );
}

#[tokio::test]
async fn a_timeout_is_retried_and_reported_as_a_timeout() {
    let server = MockServer::start().await;
    respond(&server, success().set_delay(Duration::from_secs(5)), 2).await;
    let clock = FakeClock::new();
    let policy = RetryPolicy::default()
        .with_max_retries(1)
        .with_timeout(Duration::from_millis(100));
    let transport = builder(&server, &clock)
        .retry_policy(policy)
        .build()
        .unwrap();

    let error = transport.evaluate(&request()).await.unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Timeout);
    assert_eq!(error.status(), None);
    assert_eq!(error.attempts(), 2);
    assert_eq!(clock.sleeps().len(), 1);
}

#[tokio::test]
async fn a_refused_connection_is_retried_and_reported_as_a_connection_error() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let clock = FakeClock::new();
    let transport = HttpTransport::builder(ApiKey::new(SENTINEL_KEY.to_owned()).unwrap())
        .base_url(BaseUrl::parse(&format!("http://{address}")).unwrap())
        .clock(Arc::clone(&clock) as Arc<dyn Clock>)
        .build()
        .unwrap();

    let error = transport.list_models().await.unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Connection);
    assert_eq!(error.attempts(), 3);
    assert_eq!(clock.sleeps().len(), 2);
    assert!(!error.to_string().contains(SENTINEL_KEY));
}

#[tokio::test]
async fn a_redirect_is_not_followed_so_the_key_never_reaches_another_host() {
    let elsewhere = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(success())
        .expect(0)
        .mount(&elsewhere)
        .await;
    let server = MockServer::start().await;
    let redirect = ResponseTemplate::new(307).insert_header(
        "location",
        format!("{}/v1/systemone", elsewhere.uri()).as_str(),
    );
    respond(&server, redirect, 1).await;

    let error = transport(&server, &FakeClock::new())
        .evaluate(&request())
        .await
        .unwrap_err();

    assert_eq!(error.kind(), ErrorKind::UnexpectedStatus);
    assert_eq!(error.status(), Some(307));
    assert_eq!(requests_received(&elsewhere).await, 0);
}

#[tokio::test]
async fn a_success_with_an_unreadable_body_is_an_invalid_response_that_never_quotes_the_body() {
    let bodies = [
        (format!("<html>{SENTINEL_STATE}"), "not valid JSON"),
        (
            format!(r#"{{"model":"m","answers":"{SENTINEL_STATE}"}}"#),
            "not in the documented shape",
        ),
        (
            format!(r#"{{"model":"{SENTINEL_STATE}""#),
            "ends unexpectedly",
        ),
    ];
    for (body, problem) in bodies {
        let server = MockServer::start().await;
        let garbled = ResponseTemplate::new(200)
            .insert_header("x-typesafe-request-id", "req_garbled")
            .set_body_string(body);
        respond(&server, garbled, 1).await;

        let error = transport(&server, &FakeClock::new())
            .evaluate(&request())
            .await
            .unwrap_err();

        assert_eq!(error.kind(), ErrorKind::InvalidResponse);
        assert!(error.message().contains(problem), "{error}");
        assert_eq!(error.request_id(), Some("req_garbled"));
        assert_eq!(error.attempts(), 1, "a garbled success is not retried");
        let mut rendered = format!("{error} {error:?}");
        let mut cause = std::error::Error::source(&error);
        while let Some(current) = cause {
            rendered.push_str(&current.to_string());
            cause = current.source();
        }
        assert!(
            !rendered.contains(SENTINEL_STATE),
            "the body leaked into the error: {rendered}"
        );
    }
}

#[tokio::test]
async fn models_are_listed_with_a_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", format!("Bearer {SENTINEL_KEY}").as_str()))
        .respond_with(ResponseTemplate::new(200).insert_header("x-typesafe-request-id", "req_models").set_body_json(json!({
            "models": [{ "name": "jev-latest", "description": "Latest", "release_date": "2026-09-10T18:38:01+00:00" }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let reply = transport(&server, &FakeClock::new())
        .list_models()
        .await
        .unwrap();

    assert_eq!(reply.body.models.len(), 1);
    assert_eq!(reply.body.models[0].name, "jev-latest");
    assert_eq!(reply.meta.request_id.as_deref(), Some("req_models"));
}

#[test]
fn plain_http_needs_a_loopback_host_or_the_explicit_opt_in() {
    assert!(BaseUrl::parse("http://example.com").is_err());
    assert!(BaseUrl::parse("http://127.0.0.1:4010").is_ok());
    assert!(
        BaseUrl::parse_allowing_insecure_http("http://example.com")
            .unwrap()
            .sends_key_in_clear()
    );
}

/// Collects everything a `tracing` subscriber writes.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Runs `scenario` on a single-threaded runtime with every `tracing` event, and every `log`
/// record from the HTTP stack, captured at the most verbose level.
fn capture_logs<F: Future<Output = String>>(scenario: impl FnOnce() -> F) -> (String, String) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    // Bridges `log` records (which `reqwest` and `hyper` emit) into `tracing`. It is global, so a
    // second test calling it gets an error that is safe to ignore.
    let _ = tracing_log::LogTracer::init();

    let output = tracing::subscriber::with_default(subscriber, || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(scenario())
    });
    (captured.text(), output)
}

/// Exercises a success, a retried failure and a final error, and returns everything printable.
async fn leak_scenario(log_bodies: bool) -> String {
    let server = MockServer::start().await;
    // A hostile or buggy server that echoes the credential back in its error message.
    let echo = format!("bad credentials: Bearer {SENTINEL_KEY}");
    respond(&server, api_error(500, "internal_error", &echo), 1).await;
    respond(&server, success(), 1).await;
    respond(&server, api_error(401, "authentication_error", &echo), 1).await;
    let clock = FakeClock::new();
    let builder = builder(&server, &clock).log_bodies(log_bodies);
    let printable = format!("{builder:?}");
    let transport = builder.build().unwrap();

    let reply = transport.evaluate(&request()).await.unwrap();
    let error = transport.evaluate(&request()).await.unwrap_err();

    // Every link of the source chain, rendered both ways.
    let mut causes = Vec::new();
    let mut cause: Option<&dyn std::error::Error> = Some(&error);
    while let Some(current) = cause {
        causes.push(format!("{current} {current:?}"));
        cause = current.source();
    }
    [
        printable,
        format!("{transport:?} {transport:#?} {reply:?} {error} {error:?} {error:#?}"),
        causes.join(" "),
    ]
    .join(" ")
}

#[test]
fn the_api_key_never_appears_in_debug_output_errors_or_logs_at_maximum_verbosity() {
    let (logs, printable) = capture_logs(|| leak_scenario(false));

    assert!(
        logs.contains("sending request"),
        "the scenario was traced:\n{logs}"
    );
    assert!(
        logs.contains("retrying"),
        "retry decisions are traced:\n{logs}"
    );
    assert!(logs.contains("req_ok"), "request ids are traced:\n{logs}");
    assert!(printable.contains("[REDACTED]"), "{printable}");
    for (what, text) in [("logs", &logs), ("debug and display output", &printable)] {
        assert!(
            !text.contains(SENTINEL_KEY),
            "the API key leaked into {what}:\n{text}"
        );
    }
    assert!(
        !logs.contains(SENTINEL_STATE),
        "the state was logged without body logging:\n{logs}"
    );
    assert!(
        !logs.to_ascii_lowercase().contains("bearer "),
        "an authorization value was logged:\n{logs}"
    );
}

#[test]
fn body_logging_is_opt_in_and_still_never_shows_the_key() {
    let (logs, _) = capture_logs(|| leak_scenario(true));

    assert!(
        logs.contains(SENTINEL_STATE),
        "bodies are logged once asked for:\n{logs}"
    );
    assert!(
        logs.contains("jev_client::http::body"),
        "under their own target:\n{logs}"
    );
    // Opting in to bodies is never opting in to the key, even though this mock server echoes the
    // key back inside its error body.
    assert!(
        logs.contains("bad credentials: Bearer [REDACTED]"),
        "the echo was scrubbed:\n{logs}"
    );
    assert!(
        !logs.contains(SENTINEL_KEY),
        "the API key leaked into body logs:\n{logs}"
    );
}
