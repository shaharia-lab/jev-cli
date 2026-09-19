//! Time, behind a trait so that retry behaviour can be tested without waiting.

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant, SystemTime};

/// A source of time and a way to wait.
///
/// [`HttpTransport`](crate::HttpTransport) measures latency and sleeps between retries through
/// this trait. Production code uses [`SystemClock`]; a test substitutes a clock that records the
/// requested delays and returns at once.
pub trait Clock: Send + Sync {
    /// A monotonic instant, used to measure how long a call took.
    fn now(&self) -> Instant;

    /// The wall-clock time, used only to interpret a `Retry-After` header that carries a date.
    fn wall_time(&self) -> SystemTime;

    /// Waits for `duration`.
    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// The real clock. Sleeping uses the Tokio timer, so it must run inside a Tokio runtime.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn wall_time(&self) -> SystemTime {
        SystemTime::now()
    }

    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(tokio::time::sleep(duration))
    }
}
