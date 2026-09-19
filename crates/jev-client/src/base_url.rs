//! The API root, validated so that a key is never sent somewhere it should not go.

use std::fmt;
use std::net::IpAddr;

use reqwest::Url;

/// The default API root.
const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai";

/// The root URL of the TypeSafe API, such as `https://api.typesafe.ai`.
///
/// A base URL must use `https://`. Plain `http://` is accepted for a loopback address, which is
/// what a local mock server uses, and otherwise only through
/// [`BaseUrl::parse_allowing_insecure_http`], an explicit opt-in. A URL carrying credentials, a
/// query string or a fragment is always rejected.
///
/// ```
/// use jev_client::BaseUrl;
///
/// assert_eq!(BaseUrl::default().as_str(), "https://api.typesafe.ai/");
/// assert!(BaseUrl::parse("http://127.0.0.1:8080").is_ok());
/// assert!(BaseUrl::parse("http://example.com").is_err());
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct BaseUrl {
    url: Url,
    sends_key_in_clear: bool,
}

impl BaseUrl {
    /// Parses a base URL, requiring `https://` for anything that is not a loopback address.
    ///
    /// # Errors
    ///
    /// See [`InvalidBaseUrl`]. The error never echoes the input, which could hold credentials.
    pub fn parse(input: &str) -> Result<Self, InvalidBaseUrl> {
        Self::parse_with(input, false)
    }

    /// Parses a base URL, also accepting plain `http://` to a host that is not loopback.
    ///
    /// The API key then crosses the network unencrypted. This exists for unusual test setups;
    /// a caller that uses it should warn, as [`BaseUrl::sends_key_in_clear`] allows.
    ///
    /// # Errors
    ///
    /// See [`InvalidBaseUrl`].
    pub fn parse_allowing_insecure_http(input: &str) -> Result<Self, InvalidBaseUrl> {
        Self::parse_with(input, true)
    }

    fn parse_with(input: &str, allow_insecure_http: bool) -> Result<Self, InvalidBaseUrl> {
        let mut url = Url::parse(input.trim()).map_err(|_| InvalidBaseUrl::Malformed)?;

        if !url.username().is_empty() || url.password().is_some() {
            return Err(InvalidBaseUrl::HasCredentials);
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(InvalidBaseUrl::HasQueryOrFragment);
        }
        let loopback = match url.host() {
            None => return Err(InvalidBaseUrl::Malformed),
            Some(host) => is_loopback(&host),
        };
        let sends_key_in_clear = match url.scheme() {
            "https" => false,
            "http" if loopback => false,
            "http" if allow_insecure_http => true,
            "http" => return Err(InvalidBaseUrl::InsecureHttp),
            _ => return Err(InvalidBaseUrl::UnsupportedScheme),
        };

        // A trailing slash makes `join` append to the path instead of replacing its last segment.
        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }
        Ok(Self {
            url,
            sends_key_in_clear,
        })
    }

    /// The URL as text, always ending in `/`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.url.as_str()
    }

    /// Returns `true` when requests travel over plain `http://` to a host that is not loopback,
    /// which only [`BaseUrl::parse_allowing_insecure_http`] permits.
    #[must_use]
    pub const fn sends_key_in_clear(&self) -> bool {
        self.sends_key_in_clear
    }

    /// Returns `true` when the URL uses `https://`.
    pub(crate) fn is_https(&self) -> bool {
        self.url.scheme() == "https"
    }

    /// The URL of an endpoint, given its path relative to the root (no leading slash).
    pub(crate) fn endpoint(&self, relative_path: &str) -> Result<Url, InvalidBaseUrl> {
        self.url
            .join(relative_path)
            .map_err(|_| InvalidBaseUrl::Malformed)
    }
}

impl Default for BaseUrl {
    fn default() -> Self {
        // The constant is a valid HTTPS URL; a unit test pins that down.
        Self::parse(DEFAULT_BASE_URL).unwrap_or_else(|_| unreachable!("default base URL is valid"))
    }
}

impl fmt::Debug for BaseUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "BaseUrl({})", self.url)
    }
}

impl fmt::Display for BaseUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.url.as_str())
    }
}

fn is_loopback(host: &url::Host<&str>) -> bool {
    match host {
        url::Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
        url::Host::Ipv4(address) => IpAddr::V4(*address).is_loopback(),
        url::Host::Ipv6(address) => IpAddr::V6(*address).is_loopback(),
    }
}

/// Why a string was rejected as a base URL.
///
/// The messages describe the problem without echoing the input, because a rejected URL may be one
/// that embeds a password.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum InvalidBaseUrl {
    /// Not an absolute URL with a host.
    #[error("the base URL is not a valid absolute URL such as https://api.typesafe.ai")]
    Malformed,
    /// A scheme other than `https` or `http`.
    #[error("the base URL must start with https://")]
    UnsupportedScheme,
    /// Plain `http://` to a host that is not loopback, without the explicit opt-in.
    #[error(
        "the base URL must use https://; plain http:// is accepted only for a loopback address, \
         or when insecure HTTP is explicitly allowed"
    )]
    InsecureHttp,
    /// A user name or password in the URL.
    #[error("the base URL must not contain a user name or password")]
    HasCredentials,
    /// A `?query` or `#fragment` in the URL.
    #[error("the base URL must not contain a query string or a fragment")]
    HasQueryOrFragment,
}

#[cfg(test)]
mod tests {
    use super::{BaseUrl, InvalidBaseUrl};

    #[test]
    fn the_default_is_the_public_api_over_https() {
        let base = BaseUrl::default();

        assert_eq!(base.as_str(), "https://api.typesafe.ai/");
        assert!(base.is_https());
        assert!(!base.sends_key_in_clear());
    }

    #[test]
    fn plain_http_is_rejected_unless_the_host_is_loopback() {
        assert_eq!(
            BaseUrl::parse("http://example.com"),
            Err(InvalidBaseUrl::InsecureHttp)
        );
        assert_eq!(
            BaseUrl::parse("http://192.168.1.10:8080"),
            Err(InvalidBaseUrl::InsecureHttp)
        );
        assert_eq!(
            BaseUrl::parse("http://localhost.example.com"),
            Err(InvalidBaseUrl::InsecureHttp)
        );

        for loopback in [
            "http://127.0.0.1:4010",
            "http://127.8.8.8",
            "http://[::1]:4010",
            "http://LOCALHOST:1",
        ] {
            let base = BaseUrl::parse(loopback).unwrap();
            assert!(!base.sends_key_in_clear(), "{loopback}");
        }
    }

    #[test]
    fn insecure_http_needs_the_explicit_opt_in_and_is_flagged() {
        let base = BaseUrl::parse_allowing_insecure_http("http://example.com").unwrap();

        assert!(base.sends_key_in_clear());
        assert!(
            !BaseUrl::parse_allowing_insecure_http("https://example.com")
                .unwrap()
                .sends_key_in_clear()
        );
    }

    #[test]
    fn rejects_credentials_queries_and_other_schemes_without_echoing_them() {
        let cases = [
            (
                "https://user:hunter2@example.com",
                InvalidBaseUrl::HasCredentials,
            ),
            ("https://token@example.com", InvalidBaseUrl::HasCredentials),
            (
                "https://example.com/?key=hunter2",
                InvalidBaseUrl::HasQueryOrFragment,
            ),
            (
                "https://example.com/#hunter2",
                InvalidBaseUrl::HasQueryOrFragment,
            ),
            ("ftp://example.com", InvalidBaseUrl::UnsupportedScheme),
            ("file:///etc/passwd", InvalidBaseUrl::Malformed),
            ("api.typesafe.ai", InvalidBaseUrl::Malformed),
            ("", InvalidBaseUrl::Malformed),
        ];

        for (input, expected) in cases {
            let error = BaseUrl::parse(input).unwrap_err();

            assert_eq!(error, expected, "{input}");
            assert!(!error.to_string().contains("hunter2"), "{error}");
        }
        // The opt-in relaxes the scheme rule only.
        assert_eq!(
            BaseUrl::parse_allowing_insecure_http("http://user:hunter2@example.com"),
            Err(InvalidBaseUrl::HasCredentials)
        );
    }

    #[test]
    fn endpoints_are_appended_to_the_root_and_to_a_path_prefix() {
        let root = BaseUrl::parse("https://api.typesafe.ai").unwrap();
        let prefixed = BaseUrl::parse("https://gateway.example.com/typesafe").unwrap();

        assert_eq!(
            root.endpoint("v1/systemone").unwrap().as_str(),
            "https://api.typesafe.ai/v1/systemone"
        );
        assert_eq!(
            prefixed.endpoint("v1/models").unwrap().as_str(),
            "https://gateway.example.com/typesafe/v1/models"
        );
        assert_eq!(
            prefixed.to_string(),
            "https://gateway.example.com/typesafe/"
        );
    }
}
