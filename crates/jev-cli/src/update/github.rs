//! GitHub Releases of this repository: the only place updates come from.
//!
//! Releases are read from the public REST API without credentials, and files are downloaded from
//! the repository's release download URLs, which redirect to GitHub's asset storage. Nothing else
//! is contacted: the addresses are fixed, and a redirect is followed only over HTTPS to GitHub's
//! own hosts. TLS is `rustls` with the platform verifier, built the same way as the API transport
//! in `jev-client`, and every body is read with a size cap.

use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, USER_AGENT};
use rustls_platform_verifier::BuilderVerifierExt;
use semver::Version;
use serde::Deserialize;

use super::{REPOSITORY, Release, UpdateSource};
use crate::env::Env;
use crate::error::CliError;
use crate::exit::Exit;

/// The GitHub REST API.
const API_ROOT: &str = "https://api.github.com";
/// Where release files are downloaded from.
const DOWNLOAD_ROOT: &str = "https://github.com";
/// Hosts a download may be redirected to: GitHub, and its storage for release assets.
const REDIRECT_HOSTS: [&str; 3] = [
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

/// The largest API response that is read. A release with its assets is a few kilobytes.
const MAX_API_BYTES: u64 = 4 * 1024 * 1024;
/// Time allowed to connect, and to read an API response.
const API_TIMEOUT: Duration = Duration::from_secs(30);
/// Time allowed for one whole download.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);

/// Releases of this repository on GitHub.
pub(crate) struct GitHubReleases {
    client: reqwest::Client,
    runtime: tokio::runtime::Runtime,
    api_root: String,
    download_root: String,
}

impl GitHubReleases {
    /// The real source. A build with the test hooks reads both roots from `JEV_TEST_UPDATE_URL`
    /// when it is set, so tests can serve releases locally; release builds do not contain it.
    ///
    /// # Errors
    ///
    /// An internal error when TLS or the async runtime cannot be set up.
    pub(crate) fn new(env: &Env) -> Result<Self, CliError> {
        let (api_root, download_root) = roots(env);
        let failed = |what: &str, error: &dyn std::fmt::Display| {
            CliError::internal(format!("{what} could not be set up: {error}"))
        };
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|error| failed("TLS", &error))?
        .with_platform_verifier()
        .map_err(|error| failed("the system certificate verifier", &error))?
        .with_no_client_auth();
        let agent = HeaderValue::from_static(concat!("jev/", env!("CARGO_PKG_VERSION")));
        let client = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .https_only(api_root.starts_with("https://"))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let allowed = attempt.url().scheme() == "https"
                    && attempt
                        .url()
                        .host_str()
                        .is_some_and(|host| REDIRECT_HOSTS.contains(&host));
                if attempt.previous().len() >= 5 {
                    attempt.error("too many redirects")
                } else if allowed {
                    attempt.follow()
                } else {
                    let refused = format!("refusing to follow a redirect to {}", attempt.url());
                    attempt.error(refused)
                }
            }))
            .default_headers(HeaderMap::from_iter([(USER_AGENT, agent)]))
            .connect_timeout(API_TIMEOUT)
            .read_timeout(API_TIMEOUT)
            .build()
            .map_err(|error| failed("the HTTP client", &error))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failed("the async runtime", &error))?;
        Ok(Self {
            client,
            runtime,
            api_root,
            download_root,
        })
    }

    /// One release from the API, or `None` for a 404.
    fn fetch(&self, path: &str) -> Result<Option<Release>, CliError> {
        let url = format!("{}/repos/{REPOSITORY}/{path}", self.api_root);
        tracing::debug!(%url, "asking GitHub for a release");
        let body = self.runtime.block_on(async {
            let response = self
                .client
                .get(&url)
                .header(ACCEPT, "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .timeout(API_TIMEOUT)
                .send()
                .await
                .map_err(|error| network(&error, "GitHub could not be reached"))?;
            if response.status() == StatusCode::NOT_FOUND {
                return Ok(None);
            }
            read(response, MAX_API_BYTES, "the release list")
                .await
                .map(Some)
        })?;
        let Some(body) = body else {
            return Ok(None);
        };
        let release: GitHubRelease = serde_json::from_slice(&body).map_err(|_| {
            CliError::update(
                "invalid_response",
                Exit::Internal,
                "GitHub answered with a release this jev cannot read",
            )
            .hint("try again later, or download the release by hand from its GitHub page")
        })?;
        Ok(release.into_release(&self.api_root))
    }
}

impl UpdateSource for GitHubReleases {
    fn latest(&self) -> Result<Option<Release>, CliError> {
        self.fetch("releases/latest")
    }

    fn release(&self, version: &Version) -> Result<Option<Release>, CliError> {
        Ok(self
            .fetch(&format!("releases/tags/v{version}"))?
            .filter(|release| release.version == *version))
    }

    fn download(&self, release: &Release, asset: &str, limit: u64) -> Result<Vec<u8>, CliError> {
        let url = format!(
            "{}/{REPOSITORY}/releases/download/v{}/{asset}",
            self.download_root, release.version
        );
        tracing::debug!(%url, "downloading");
        self.runtime.block_on(async {
            let response = self
                .client
                .get(&url)
                .timeout(DOWNLOAD_TIMEOUT)
                .send()
                .await
                .map_err(|error| network(&error, &format!("{asset} could not be downloaded")))?;
            read(response, limit, asset).await
        })
    }
}

/// The API and download roots.
fn roots(env: &Env) -> (String, String) {
    #[cfg(feature = "internal-test-hooks")]
    if let Some(root) = env.get("JEV_TEST_UPDATE_URL") {
        let root = root.trim_end_matches('/').to_owned();
        return (root.clone(), root);
    }
    let _ = env;
    (API_ROOT.to_owned(), DOWNLOAD_ROOT.to_owned())
}

/// Reads a successful response's body, refusing more than `limit` bytes.
async fn read(
    mut response: reqwest::Response,
    limit: u64,
    what: &str,
) -> Result<Vec<u8>, CliError> {
    let status = response.status();
    if !status.is_success() {
        return Err(refused(status, what));
    }
    let too_large = || {
        CliError::update(
            "invalid_response",
            Exit::Internal,
            format!("{what} is larger than {limit} bytes"),
        )
        .hint("nothing was changed; try again later")
    };
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        return Err(too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| network(&error, &format!("the download of {what} was interrupted")))?
    {
        if (body.len() + chunk.len()) as u64 > limit {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// A failure to reach GitHub or to finish reading from it.
fn network(error: &reqwest::Error, what: &str) -> CliError {
    let (code, message) = if error.is_timeout() {
        ("timeout", format!("{what}: timed out"))
    } else {
        ("connection", format!("{what}: {}", describe(error)))
    };
    let mut failure = CliError::update(code, Exit::Network, message).hint(
        "check the network connection and any proxy settings, then try again; nothing was changed",
    );
    failure.retryable = true;
    failure
}

/// The innermost cause of a `reqwest` error, which is the part worth reading.
fn describe(error: &reqwest::Error) -> String {
    let mut source: &dyn std::error::Error = error;
    while let Some(inner) = source.source() {
        source = inner;
    }
    source.to_string()
}

/// An unsuccessful status from GitHub.
fn refused(status: StatusCode, what: &str) -> CliError {
    let mut error = if status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS {
        CliError::update(
            "rate_limited",
            Exit::RateLimited,
            format!("GitHub refused to send {what} (HTTP {status})"),
        )
        .hint("GitHub allows 60 unauthenticated requests an hour from one address; try again later")
    } else {
        CliError::update(
            "server_error",
            Exit::Network,
            format!("GitHub failed to send {what} (HTTP {status})"),
        )
        .hint("try again later; nothing was changed")
    };
    error.http_status = Some(status.as_u16());
    error.retryable = true;
    error
}

/// A release as the GitHub API describes it. Only what the updater needs is read.
#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
}

impl GitHubRelease {
    /// The release, unless it is a draft, a pre-release, or tagged with something other than
    /// `v<version>`.
    fn into_release(self, api_root: &str) -> Option<Release> {
        if self.draft || self.prerelease {
            return None;
        }
        let version = Version::parse(self.tag_name.strip_prefix('v')?).ok()?;
        if !version.pre.is_empty() {
            return None;
        }
        let url = self.html_url.unwrap_or_else(|| {
            let root = if api_root == API_ROOT {
                DOWNLOAD_ROOT
            } else {
                api_root
            };
            format!("{root}/{REPOSITORY}/releases/tag/v{version}")
        });
        Some(Release {
            version,
            url,
            assets: self.assets.into_iter().map(|asset| asset.name).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::GitHubRelease;

    fn parse(json: &str) -> Option<super::Release> {
        serde_json::from_str::<GitHubRelease>(json)
            .unwrap()
            .into_release(super::API_ROOT)
    }

    #[test]
    fn a_release_is_read_tolerantly_and_only_when_it_is_stable() {
        let release = parse(
            r#"{"tag_name":"v0.2.0","html_url":"https://github.com/x/releases/tag/v0.2.0",
                "assets":[{"name":"SHA256SUMS","size":10,"extra":true}],"unknown":1}"#,
        )
        .unwrap();
        assert_eq!(release.version, Version::new(0, 2, 0));
        assert_eq!(release.assets, ["SHA256SUMS"]);

        assert_eq!(
            parse(r#"{"tag_name":"v0.2.0"}"#).unwrap().url,
            "https://github.com/shaharia-lab/jev-cli/releases/tag/v0.2.0"
        );
        for skipped in [
            r#"{"tag_name":"v0.2.0","draft":true}"#,
            r#"{"tag_name":"v0.2.0","prerelease":true}"#,
            r#"{"tag_name":"v0.3.0-rc.1"}"#,
            r#"{"tag_name":"0.2.0"}"#,
            r#"{"tag_name":"vnext"}"#,
        ] {
            assert_eq!(parse(skipped), None, "{skipped}");
        }
    }
}
