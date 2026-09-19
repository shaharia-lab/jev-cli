//! Pacing shared by many calls, so that one call being rate limited slows all of them.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::clock::Clock;

/// The gap between call starts after the first slow-down.
const FIRST_SPACING: Duration = Duration::from_millis(100);

/// The widest gap between call starts.
const MAX_SPACING: Duration = Duration::from_secs(5);

/// Below this gap, pacing stops altogether.
const MIN_SPACING: Duration = Duration::from_millis(10);

/// Pacing shared by every call through the transports that hold it.
///
/// Retries alone slow down only the call that was refused: the others keep sending at full speed
/// and are refused in turn. A `Throttle` shared by concurrent calls makes a 429 or 529 on any of
/// them slow all of them, the way a well-behaved client pool should:
///
/// - **No call starts before the server's delay is over.** The delay is the `Retry-After` the
///   server sent, or else the retry policy's backoff.
/// - **Calls then start one at a time, spaced out.** The gap doubles with each refusal, from
///   100 ms up to 5 s, and shrinks by an eighth with each success until pacing stops: a pool
///   that keeps being refused slows down fast, and one that stops being refused speeds up again
///   gradually.
///
/// Without refusals it does nothing. Attach one with
/// [`HttpTransport::with_throttle`](crate::HttpTransport::with_throttle); clones of a transport
/// share it.
#[derive(Debug, Default)]
pub struct Throttle {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    /// The gap between call starts; zero when calls are not paced.
    spacing: Duration,
    /// No call starts before this.
    next_start: Option<Instant>,
}

impl Throttle {
    /// A throttle that does not slow anything down until a call is refused.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current gap between call starts; zero when calls are not paced.
    #[must_use]
    pub fn spacing(&self) -> Duration {
        self.lock().spacing
    }

    /// Waits until this call may start, and books the next start after it.
    pub(crate) async fn wait(&self, clock: &dyn Clock) {
        let now = clock.now();
        let start = {
            let mut state = self.lock();
            let start = state.next_start.map_or(now, |next| next.max(now));
            state.next_start = if state.spacing.is_zero() && start == now {
                None
            } else {
                Some(start + state.spacing)
            };
            start
        };
        if start > now {
            clock.sleep(start.duration_since(now)).await;
        }
    }

    /// A call was refused with 429 or 529: nothing starts before `delay` from `now`, and calls
    /// are spaced further apart.
    pub(crate) fn slow_down(&self, now: Instant, delay: Duration) {
        let mut state = self.lock();
        let wider = state.spacing * 2;
        state.spacing = wider.clamp(FIRST_SPACING, MAX_SPACING);
        let resume = now + delay;
        state.next_start = Some(state.next_start.map_or(resume, |next| next.max(resume)));
    }

    /// A call succeeded: calls may be spaced a little closer together.
    pub(crate) fn speed_up(&self) {
        let mut state = self.lock();
        let narrower = state.spacing.saturating_sub(state.spacing / 8);
        state.spacing = if narrower < MIN_SPACING {
            Duration::ZERO
        } else {
            narrower
        };
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        // The state is two plain values, valid after any panic, so a poisoned lock is still usable.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Mutex;
    use std::time::{Duration, Instant, SystemTime};

    use super::Throttle;
    use crate::clock::Clock;

    /// A clock that stands still, even when slept on, and remembers every sleep.
    struct Frozen {
        origin: Instant,
        sleeps: Mutex<Vec<Duration>>,
    }

    impl Frozen {
        fn new() -> Self {
            Self {
                origin: Instant::now(),
                sleeps: Mutex::new(Vec::new()),
            }
        }

        fn take_sleeps(&self) -> Vec<Duration> {
            std::mem::take(&mut *self.sleeps.lock().unwrap())
        }
    }

    impl Clock for Frozen {
        fn now(&self) -> Instant {
            self.origin
        }

        fn wall_time(&self) -> SystemTime {
            SystemTime::UNIX_EPOCH
        }

        fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            self.sleeps.lock().unwrap().push(duration);
            Box::pin(async {})
        }
    }

    fn block_on(future: impl Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future);
    }

    #[test]
    fn without_a_refusal_no_call_waits() {
        let (throttle, clock) = (Throttle::new(), Frozen::new());

        block_on(async {
            for _ in 0..5 {
                throttle.wait(&clock).await;
            }
        });

        assert!(clock.take_sleeps().is_empty());
        assert_eq!(throttle.spacing(), Duration::ZERO);
    }

    #[test]
    fn a_refusal_holds_every_call_until_the_delay_is_over_then_spaces_them_out() {
        let (throttle, clock) = (Throttle::new(), Frozen::new());

        throttle.slow_down(clock.now(), Duration::from_secs(2));
        block_on(async {
            for _ in 0..3 {
                throttle.wait(&clock).await;
            }
        });

        let ms = Duration::from_millis;
        assert_eq!(clock.take_sleeps(), [ms(2000), ms(2100), ms(2200)]);
    }

    #[test]
    fn refusals_widen_the_gap_up_to_a_limit_and_successes_narrow_it_to_nothing() {
        let (throttle, clock) = (Throttle::new(), Frozen::new());

        let mut widths = Vec::new();
        for _ in 0..8 {
            throttle.slow_down(clock.now(), Duration::ZERO);
            widths.push(throttle.spacing().as_millis());
        }
        let mut successes = 0;
        while !throttle.spacing().is_zero() {
            throttle.speed_up();
            successes += 1;
        }

        assert_eq!(widths, [100, 200, 400, 800, 1600, 3200, 5000, 5000]);
        assert_eq!(successes, 47, "about fifty successes undo the widest gap");
    }

    #[test]
    fn a_later_refusal_never_shortens_a_pause_already_asked_for() {
        let (throttle, clock) = (Throttle::new(), Frozen::new());

        throttle.slow_down(clock.now(), Duration::from_secs(10));
        throttle.slow_down(clock.now(), Duration::from_secs(1));
        block_on(throttle.wait(&clock));

        assert_eq!(clock.take_sleeps(), [Duration::from_secs(10)]);
    }
}
