//! Builds the HTTP transport from the effective settings.

use jev_client::{ApiKey, BaseUrl, HttpTransport, RetryPolicy};

use crate::config::{Key, Settings, Value};
use crate::error::CliError;
use crate::notice::Notice;

/// Choices about the connection that are flags rather than settings, because each one weakens a
/// protection and should be a deliberate act on a single command.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Connection {
    /// Allow plain `http://` to a host that is not loopback.
    pub(crate) insecure_allow_http: bool,
    /// Trace request and response bodies.
    pub(crate) debug_bodies: bool,
}

/// The transport, and the warnings its configuration deserves.
///
/// # Errors
///
/// A usage error when the base URL is not acceptable.
pub(crate) fn transport(
    settings: &Settings,
    connection: Connection,
    api_key: ApiKey,
) -> Result<(HttpTransport, Vec<Notice>), CliError> {
    let base_url = base_url(settings, connection.insecure_allow_http)?;
    let mut notices = Vec::new();
    if base_url.sends_key_in_clear() {
        notices.push(
            Notice::warning(
                "insecure_http",
                format!("the API key is being sent unencrypted to {base_url}"),
            )
            .hint("use an https:// base URL; --insecure-allow-http is for test setups only"),
        );
    }
    if connection.debug_bodies {
        notices.push(
            Notice::warning(
                "debug_bodies",
                "request and response bodies are being logged; `state` may contain sensitive data",
            )
            .hint("drop --debug-bodies before sharing logs"),
        );
    }

    let mut policy = RetryPolicy::default();
    if let Some(Value::Duration(timeout)) = settings.get(Key::Timeout).value {
        policy = policy.with_timeout(timeout);
    }
    if let Some(Value::Count(max_retries)) = settings.get(Key::MaxRetries).value {
        policy = policy.with_max_retries(max_retries);
    }

    let transport = HttpTransport::builder(api_key)
        .base_url(base_url)
        .retry_policy(policy)
        .user_agent(concat!("jev/", env!("CARGO_PKG_VERSION")))
        .log_bodies(connection.debug_bodies)
        .build()?;
    Ok((transport, notices))
}

fn base_url(settings: &Settings, insecure_allow_http: bool) -> Result<BaseUrl, CliError> {
    let setting = settings.get(Key::BaseUrl);
    let text = setting
        .value
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    let parsed = if insecure_allow_http {
        BaseUrl::parse_allowing_insecure_http(&text)
    } else {
        BaseUrl::parse(&text)
    };
    parsed.map_err(|error| {
        let origin = setting.source.origin().map_or_else(|| "the base URL".to_owned(), |origin| format!("the base URL from {origin}"));
        CliError::usage(format!("{origin} is not acceptable: {error}"))
            .hint("use an https:// URL; plain http:// works for 127.0.0.1 and localhost, or with --insecure-allow-http")
    })
}
