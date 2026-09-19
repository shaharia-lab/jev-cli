//! When to retry, and how long to wait first.

use std::time::{Duration, SystemTime};

use reqwest::header::{HeaderMap, RETRY_AFTER};

/// Statuses that are never retried, whatever the policy says: the request itself is at fault, so
/// sending it again can only fail the same way (and, for 401/403, hammer a rejected credential).
const NEVER_RETRIED: [u16; 5] = [400, 401, 403, 404, 422];

/// The non-standard header carrying a retry delay in milliseconds, honoured like the SDKs do.
const RETRY_AFTER_MS: &str = "retry-after-ms";

/// How failed requests are retried.
///
/// The defaults match the official TypeSafe SDKs: 2 retries, a 0.5 s first delay doubling up to
/// 5 s, a quarter of each delay randomly subtracted, a 30 s timeout per attempt, and a server's
/// `Retry-After` honoured up to 60 s. Retried by default are 408, 429, every 5xx (so 529 too),
/// connection failures and timeouts.
///
/// ```
/// use std::time::Duration;
/// use jev_client::RetryPolicy;
///
/// let patient = RetryPolicy::default().with_max_retries(5).with_timeout(Duration::from_secs(60));
/// assert_eq!(patient.max_retries, 5);
///
/// let never = RetryPolicy::none();
/// assert_eq!(never.max_retries, 0);
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct RetryPolicy {
    /// Retries after the first attempt. `0` disables retrying.
    pub max_retries: u32,
    /// The delay before the first retry. Each later delay doubles, up to `backoff_max`.
    pub backoff_initial: Duration,
    /// The longest backoff delay.
    pub backoff_max: Duration,
    /// The fraction of each backoff delay that is randomly subtracted, from 0 to 1. Spreading
    /// retries out stops many clients from retrying in step.
    pub backoff_jitter: f64,
    /// The time allowed for one attempt, from sending the request to reading the whole response.
    pub timeout: Duration,
    /// Whether a `Retry-After` or `retry-after-ms` response header sets the delay.
    pub respect_retry_after: bool,
    /// The longest server-requested delay that is honoured. A longer one falls back to backoff.
    pub max_retry_after: Duration,
    /// Whether a failure to connect, or a connection lost mid-response, is retried.
    pub retry_connection_errors: bool,
    /// Whether an attempt that timed out is retried.
    pub retry_timeouts: bool,
    /// The HTTP statuses that are retried. 400, 401, 403, 404 and 422 are never retried, even
    /// when listed here.
    pub retry_statuses: Vec<u16>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            backoff_initial: Duration::from_millis(500),
            backoff_max: Duration::from_secs(5),
            backoff_jitter: 0.25,
            timeout: Duration::from_secs(30),
            respect_retry_after: true,
            max_retry_after: Duration::from_secs(60),
            retry_connection_errors: true,
            retry_timeouts: true,
            retry_statuses: [408, 429].into_iter().chain(500..=599).collect(),
        }
    }
}

impl RetryPolicy {
    /// The default policy with retrying switched off. The per-attempt timeout still applies.
    #[must_use]
    pub fn none() -> Self {
        Self::default().with_max_retries(0)
    }

    /// Sets the number of retries after the first attempt.
    #[must_use]
    pub const fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Sets the first backoff delay and the longest one.
    #[must_use]
    pub const fn with_backoff(mut self, initial: Duration, max: Duration) -> Self {
        self.backoff_initial = initial;
        self.backoff_max = max;
        self
    }

    /// Sets the fraction of each backoff delay that is randomly subtracted. Values outside 0 to 1,
    /// and NaN, are clamped into range when a delay is computed.
    #[must_use]
    pub const fn with_jitter(mut self, fraction: f64) -> Self {
        self.backoff_jitter = fraction;
        self
    }

    /// Sets the time allowed for one attempt.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Returns `true` when a response with `status` should be retried under this policy.
    #[must_use]
    pub fn retries_status(&self, status: u16) -> bool {
        !NEVER_RETRIED.contains(&status) && self.retry_statuses.contains(&status)
    }

    /// The backoff delay before retry number `retry` (0 for the first retry).
    ///
    /// `random` is a sample from 0 to 1 that decides how much of the jitter is applied; passing it
    /// in keeps this function pure.
    #[must_use]
    pub fn backoff_delay(&self, retry: u32, random: f64) -> Duration {
        let doubled = self
            .backoff_initial
            .saturating_mul(2_u32.saturating_pow(retry));
        let capped = doubled.min(self.backoff_max);

        let jitter = clamp_unit(self.backoff_jitter);
        capped.mul_f64(1.0 - jitter * clamp_unit(random))
    }

    /// The delay before the next retry: the server's, when there is an acceptable one, and
    /// otherwise backoff.
    pub(crate) fn delay(
        &self,
        retry: u32,
        server_delay: Option<Duration>,
        random: f64,
    ) -> Duration {
        server_delay
            .filter(|delay| self.respect_retry_after && *delay <= self.max_retry_after)
            .unwrap_or_else(|| self.backoff_delay(retry, random))
    }
}

/// Clamps to the range 0 to 1, reading NaN as 0.
fn clamp_unit(value: f64) -> f64 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// The delay a server asked for, from `retry-after-ms` or `Retry-After`.
///
/// `Retry-After` may be a number of seconds or an HTTP date; a date is measured from `now`. A value
/// that cannot be read, or that is negative, yields `None`.
pub(crate) fn server_delay(headers: &HeaderMap, now: SystemTime) -> Option<Duration> {
    let text = |name: &str| headers.get(name)?.to_str().ok().map(str::trim);

    if let Some(milliseconds) = text(RETRY_AFTER_MS).and_then(|value| value.parse::<f64>().ok()) {
        return duration_from_secs(milliseconds / 1000.0);
    }

    let value = text(RETRY_AFTER.as_str())?;
    if let Ok(seconds) = value.parse::<f64>() {
        return duration_from_secs(seconds);
    }
    let date = httpdate::parse_http_date(value).ok()?;
    Some(date.duration_since(now).unwrap_or(Duration::ZERO))
}

fn duration_from_secs(seconds: f64) -> Option<Duration> {
    Duration::try_from_secs_f64(seconds).ok()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use reqwest::header::{HeaderMap, HeaderValue};

    use super::{RetryPolicy, server_delay};

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        pairs
            .iter()
            .map(|&(name, value)| (name.parse().unwrap(), HeaderValue::from_static(value)))
            .collect()
    }

    #[test]
    fn the_defaults_match_the_official_sdks() {
        let policy = RetryPolicy::default();

        assert_eq!(policy.max_retries, 2);
        assert_eq!(policy.backoff_initial, Duration::from_millis(500));
        assert_eq!(policy.backoff_max, Duration::from_secs(5));
        assert_eq!(policy.backoff_jitter.to_bits(), 0.25_f64.to_bits());
        assert_eq!(policy.timeout, Duration::from_secs(30));
        assert_eq!(policy.max_retry_after, Duration::from_secs(60));
        assert!(
            policy.respect_retry_after && policy.retry_connection_errors && policy.retry_timeouts
        );
    }

    #[test]
    fn retries_408_429_and_every_5xx_including_529() {
        let policy = RetryPolicy::default();

        for status in [408, 429, 500, 502, 503, 504, 529, 599] {
            assert!(policy.retries_status(status), "{status}");
        }
        for status in [200, 302, 400, 401, 403, 404, 409, 422, 499] {
            assert!(!policy.retries_status(status), "{status}");
        }
    }

    #[test]
    fn request_errors_are_never_retried_even_when_the_policy_lists_them() {
        let policy = RetryPolicy {
            retry_statuses: vec![400, 401, 403, 404, 409, 422],
            ..RetryPolicy::default()
        };

        for status in [400, 401, 403, 404, 422] {
            assert!(!policy.retries_status(status), "{status}");
        }
        assert!(policy.retries_status(409));
    }

    #[test]
    fn backoff_doubles_from_half_a_second_up_to_the_cap() {
        let policy = RetryPolicy::default().with_jitter(0.0);

        let delays: Vec<u128> = (0..6)
            .map(|retry| policy.backoff_delay(retry, 0.5).as_millis())
            .collect();

        assert_eq!(delays, [500, 1000, 2000, 4000, 5000, 5000]);
        assert_eq!(
            policy.backoff_delay(u32::MAX, 0.5),
            Duration::from_secs(5),
            "no overflow"
        );
    }

    #[test]
    fn jitter_only_ever_subtracts_up_to_its_fraction() {
        let policy = RetryPolicy::default();

        assert_eq!(policy.backoff_delay(0, 0.0), Duration::from_millis(500));
        assert_eq!(policy.backoff_delay(0, 1.0), Duration::from_millis(375));
        assert_eq!(policy.backoff_delay(0, 0.5), Duration::from_micros(437_500));
    }

    #[test]
    fn nonsense_jitter_and_random_values_are_clamped() {
        let full = RetryPolicy::default().with_jitter(7.0);
        let negative = RetryPolicy::default().with_jitter(-1.0);
        let nan = RetryPolicy::default().with_jitter(f64::NAN);

        assert_eq!(full.backoff_delay(0, 1.0), Duration::ZERO);
        assert_eq!(negative.backoff_delay(0, 1.0), Duration::from_millis(500));
        assert_eq!(nan.backoff_delay(0, 1.0), Duration::from_millis(500));
        assert_eq!(
            RetryPolicy::default().backoff_delay(0, f64::NAN),
            Duration::from_millis(500)
        );
        assert_eq!(
            RetryPolicy::default().backoff_delay(0, -3.0),
            Duration::from_millis(500)
        );
    }

    #[test]
    fn reads_the_server_delay_in_seconds_milliseconds_or_as_a_date() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000); // 2001-09-09T01:46:40Z

        assert_eq!(
            server_delay(&headers(&[("retry-after", "1")]), now),
            Some(Duration::from_secs(1))
        );
        assert_eq!(
            server_delay(&headers(&[("retry-after", " 2.5 ")]), now),
            Some(Duration::from_millis(2500))
        );
        assert_eq!(
            server_delay(&headers(&[("retry-after-ms", "250")]), now),
            Some(Duration::from_millis(250))
        );
        assert_eq!(
            server_delay(
                &headers(&[("retry-after", "Sun, 09 Sep 2001 01:46:50 GMT")]),
                now
            ),
            Some(Duration::from_secs(10))
        );
        assert_eq!(
            server_delay(
                &headers(&[("retry-after", "Sun, 09 Sep 2001 01:00:00 GMT")]),
                now
            ),
            Some(Duration::ZERO),
            "a date in the past means now"
        );
    }

    #[test]
    fn milliseconds_win_over_seconds_and_unreadable_values_are_ignored() {
        let now = SystemTime::UNIX_EPOCH;

        let both = headers(&[("retry-after", "30"), ("retry-after-ms", "100")]);
        assert_eq!(server_delay(&both, now), Some(Duration::from_millis(100)));

        for value in ["soon", "-1", "NaN", "inf", ""] {
            assert_eq!(
                server_delay(&headers(&[("retry-after", value)]), now),
                None,
                "{value:?}"
            );
        }
        assert_eq!(server_delay(&HeaderMap::new(), now), None);
    }

    #[test]
    fn an_acceptable_server_delay_replaces_backoff_and_an_excessive_one_does_not() {
        let policy = RetryPolicy::default().with_jitter(0.0);

        assert_eq!(
            policy.delay(0, Some(Duration::from_secs(1)), 0.0),
            Duration::from_secs(1)
        );
        assert_eq!(
            policy.delay(0, Some(Duration::from_secs(60)), 0.0),
            Duration::from_secs(60)
        );
        assert_eq!(
            policy.delay(1, Some(Duration::from_secs(61)), 0.0),
            Duration::from_secs(1)
        );
        assert_eq!(policy.delay(1, None, 0.0), Duration::from_secs(1));

        let deaf = RetryPolicy {
            respect_retry_after: false,
            ..policy
        };
        assert_eq!(
            deaf.delay(0, Some(Duration::from_secs(9)), 0.0),
            Duration::from_millis(500)
        );
    }
}
