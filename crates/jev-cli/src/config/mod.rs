//! Configuration: where it lives, how it is read, and how it is changed safely.

mod file;
mod paths;
mod settings;

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime};

pub(crate) use file::ConfigFile;
pub(crate) use paths::{CONFIG_FILE, config_dir};
pub(crate) use settings::{DEFAULT_PROFILE, Flags, Key, Setting, Settings, Source, Value};

use crate::error::CliError;

/// How long another `jev` process may hold the lock, without the configuration changing, before
/// giving up. The only holder is another short `jev` write, but each one syncs the file to disk,
/// which can take seconds on a busy disk; failing a `config set` for that would be worse than waiting.
const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// The first and the longest pause between two attempts to take the lock.
const LOCK_POLL_MIN: Duration = Duration::from_millis(10);
const LOCK_POLL_MAX: Duration = Duration::from_millis(100);

/// The configuration directory and the file in it.
#[derive(Clone, Debug)]
pub(crate) struct ConfigStore {
    dir: PathBuf,
}

impl ConfigStore {
    pub(crate) const fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    pub(crate) fn file_path(&self) -> PathBuf {
        self.dir.join(CONFIG_FILE)
    }

    /// Reads the configuration. A file that does not exist is an empty configuration.
    ///
    /// # Errors
    ///
    /// A usage error when the file cannot be read, or is not a valid configuration.
    pub(crate) fn load(&self) -> Result<ConfigFile, CliError> {
        let path = self.file_path();
        match fs::read_to_string(&path) {
            Ok(text) => ConfigFile::parse(&text, &path.display().to_string()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(ConfigFile::default()),
            Err(error) => Err(
                CliError::usage(format!("cannot read {}: {error}", path.display())).hint(
                    "check the file's permissions, or point JEV_CONFIG_DIR at another directory",
                ),
            ),
        }
    }

    /// Changes the configuration: reads it, applies `change`, and writes it back, all while
    /// holding a lock, so that two `jev` processes cannot lose each other's changes.
    ///
    /// The new content goes to a temporary file that is then renamed over the old one. A reader
    /// therefore sees the old file or the new one, never half of either, and a crash cannot leave a
    /// truncated file behind.
    ///
    /// # Errors
    ///
    /// Whatever `change` fails with (nothing is written then), or a usage error when the directory
    /// cannot be written to or the lock cannot be had in time.
    pub(crate) fn update<T>(
        &self,
        change: impl FnOnce(&mut ConfigFile) -> Result<T, CliError>,
    ) -> Result<T, CliError> {
        let cannot_write = |what: &str, error: &io::Error| {
            CliError::usage(format!("cannot {what} {}: {error}", self.dir.display()))
                .hint("check the directory's permissions, or point JEV_CONFIG_DIR at a directory jev may write to")
        };
        fs::create_dir_all(&self.dir).map_err(|error| cannot_write("create", &error))?;

        let _lock = self.lock(LOCK_TIMEOUT).map_err(|error| {
            if error.kind() == io::ErrorKind::TimedOut {
                CliError::usage(error.to_string()).hint(
                    "wait for the other jev process to finish and try again; the lock is released when it exits",
                )
            } else {
                cannot_write("lock the configuration in", &error)
            }
        })?;
        let mut file = self.load()?;
        let outcome = change(&mut file)?;
        write_atomically(&self.file_path(), file.to_toml().as_bytes())
            .map_err(|error| cannot_write("write to", &error))?;
        Ok(outcome)
    }

    /// Takes the exclusive lock, which is released when the returned file is dropped. The lock is
    /// on a file of its own, because the configuration file itself is replaced on every write.
    ///
    /// Waiters poll with a jittered, growing pause, so that a crowd of them started together does
    /// not keep retrying in step and missing the moment the lock is released. The timeout counts
    /// from the last time the configuration file changed, not from the first attempt: a queue of
    /// writers that keeps moving is waited for however long it is, and only a holder that makes
    /// no progress for the whole timeout is given up on.
    fn lock(&self, timeout: Duration) -> io::Result<File> {
        self.lock_with(timeout, &mut RealWaiter::new(self.file_path()))
    }

    /// [`lock`](Self::lock), with the passage of time and the sight of other writers behind a
    /// [`Waiter`], so that a test can drive both exactly instead of racing a loaded machine.
    fn lock_with(&self, timeout: Duration, waiter: &mut impl Waiter) -> io::Result<File> {
        let path = self.dir.join("config.lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)?;
        let mut seen = waiter.progress();
        let mut started = waiter.now();
        let mut jitter = Jitter::new();
        let mut pause = LOCK_POLL_MIN;
        loop {
            match lock.try_lock() {
                Ok(()) => return Ok(lock),
                Err(fs::TryLockError::WouldBlock)
                    if waiter.now().duration_since(started) < timeout =>
                {
                    waiter.sleep(jitter.between(pause / 2, pause));
                    pause = (pause * 2).min(LOCK_POLL_MAX);
                    let now = waiter.progress();
                    if now != seen {
                        seen = now;
                        started = waiter.now();
                    }
                }
                Err(fs::TryLockError::WouldBlock) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!(
                            "gave up after {} s: another jev process has held {} all that time",
                            timeout.as_secs_f32(),
                            path.display()
                        ),
                    ));
                }
                Err(fs::TryLockError::Error(error)) => return Err(error),
            }
        }
    }
}

/// What the last write to the configuration left behind. Two writes are told apart by when the
/// file was last changed and how long it is; a queue of writers changes at least one of the two.
#[derive(Debug, PartialEq, Eq)]
struct Written {
    modified: Option<SystemTime>,
    len: u64,
}

/// Everything the waiting half of [`ConfigStore::lock_with`] takes from the outside world: the
/// clock it measures the timeout on, the pause between two attempts, and the sight of another
/// writer making progress.
trait Waiter {
    /// A monotonic instant.
    fn now(&mut self) -> Instant;

    /// Waits for `pause` before the next attempt.
    fn sleep(&mut self, pause: Duration);

    /// The state of the configuration file, or `None` while there is no readable file.
    fn progress(&mut self) -> Option<Written>;
}

/// The real thing: the system clock, a real sleep, and the configuration file on disk.
struct RealWaiter {
    config: PathBuf,
}

impl RealWaiter {
    const fn new(config: PathBuf) -> Self {
        Self { config }
    }
}

impl Waiter for RealWaiter {
    fn now(&mut self) -> Instant {
        Instant::now()
    }

    fn sleep(&mut self, pause: Duration) {
        std::thread::sleep(pause);
    }

    fn progress(&mut self) -> Option<Written> {
        fs::metadata(&self.config).ok().map(|metadata| Written {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        })
    }
}

/// A cheap pseudo-random source for spreading out lock retries; it needs to differ between
/// processes and calls, not to be unpredictable.
struct Jitter(u64);

impl Jitter {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.subsec_nanos());
        // xorshift must not start from zero.
        Self((u64::from(nanos) << 32 | u64::from(std::process::id())) | 1)
    }

    /// A duration in `low..=high`.
    fn between(&mut self, low: Duration, high: Duration) -> Duration {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        let span = u64::try_from(high.saturating_sub(low).as_micros()).unwrap_or(u64::MAX);
        low + Duration::from_micros(self.0 % span.saturating_add(1))
    }
}

/// Writes `content` to `path` by way of a temporary file in the same directory and a rename.
fn write_atomically(path: &Path, content: &[u8]) -> io::Result<()> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = format!(
        ".{CONFIG_FILE}.{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let temporary = path.with_file_name(unique);

    let written = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(content)?;
        // Make sure the bytes are on disk before the rename makes them the configuration.
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    use super::{ConfigStore, Jitter, Key, Value, Waiter, Written};

    /// The timeout the lock tests are written against. Nothing sleeps for it: [`Waiting`] hands
    /// the loop a clock of its own, so a long, realistic timeout costs a few hundred iterations.
    const TIMEOUT: Duration = super::LOCK_TIMEOUT;

    /// A [`Waiter`] on a clock of its own. Time passes only when the lock loop asks to sleep, and
    /// the writer it is waiting behind makes its progress on that same clock, so the test cannot
    /// race the machine it runs on.
    struct Waiting {
        /// The origin of the clock: `Instant` has no constructor of its own, and only differences
        /// from this point are ever read.
        origin: Instant,
        elapsed: Duration,
        /// Writes the queue has still to make, and how long each one takes.
        left: u32,
        every: Duration,
        since: Duration,
        /// Stands in for the file the queue appends to: every write makes it one byte longer.
        len: u64,
        /// The lock the queue holds, released once the last writer is done.
        held: Option<File>,
    }

    impl Waiting {
        /// A queue of `writers` that each take `every` to finish, and then let the lock go.
        fn queue(held: File, writers: u32, every: Duration) -> Self {
            Self {
                origin: Instant::now(),
                elapsed: Duration::ZERO,
                left: writers,
                every,
                since: Duration::ZERO,
                len: 0,
                held: Some(held),
            }
        }

        /// A holder that never writes anything and never lets the lock go.
        fn stuck() -> Self {
            Self {
                origin: Instant::now(),
                elapsed: Duration::ZERO,
                left: 0,
                every: Duration::MAX,
                since: Duration::ZERO,
                len: 0,
                held: None,
            }
        }

        /// How long the lock loop has waited, on this clock.
        fn elapsed(&self) -> Duration {
            self.elapsed
        }
    }

    impl Waiter for Waiting {
        fn now(&mut self) -> Instant {
            self.origin + self.elapsed
        }

        fn sleep(&mut self, pause: Duration) {
            self.elapsed += pause;
            self.since += pause;
            while self.left > 0 && self.since >= self.every {
                self.since -= self.every;
                self.left -= 1;
                self.len += 1;
            }
            if self.left == 0 {
                self.held = None;
            }
        }

        fn progress(&mut self) -> Option<Written> {
            (self.len > 0).then_some(Written {
                modified: None,
                len: self.len,
            })
        }
    }

    /// A fresh directory under the build's temporary directory.
    fn scratch(name: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "jev-config-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_missing_file_is_an_empty_configuration_and_the_first_write_creates_everything() {
        let store = ConfigStore::new(scratch("first-write").join("nested").join("jev"));
        assert!(store.load().unwrap().profiles.is_empty());

        store
            .update(|file| {
                file.set("default", Key::Model, &Value::Text("jev-1.13.0".into()));
                Ok(())
            })
            .unwrap();

        let reread = store.load().unwrap();
        assert_eq!(
            reread.profiles["default"][&Key::Model],
            Value::Text("jev-1.13.0".into())
        );
        let leftovers: Vec<String> = fs::read_dir(store.dir())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| {
                std::path::Path::new(name)
                    .extension()
                    .is_some_and(|extension| extension == "tmp")
            })
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn a_failed_change_writes_nothing() {
        let store = ConfigStore::new(scratch("failed-change"));
        store
            .update(|file| {
                file.set("default", Key::Model, &Value::Text("kept".into()));
                Ok(())
            })
            .unwrap();
        let before = fs::read_to_string(store.file_path()).unwrap();

        let outcome: Result<(), _> = store.update(|file| {
            file.set("default", Key::Model, &Value::Text("discarded".into()));
            Err(crate::error::CliError::usage("changed my mind"))
        });

        assert!(outcome.is_err());
        assert_eq!(fs::read_to_string(store.file_path()).unwrap(), before);
    }

    #[test]
    fn concurrent_updates_in_one_process_do_not_lose_each_other() {
        let store = ConfigStore::new(scratch("threads"));
        let writers: Vec<_> = (0..8)
            .map(|index| {
                let store = store.clone();
                std::thread::spawn(move || {
                    store
                        .update(|file| {
                            file.create_profile(&format!("p{index}"));
                            Ok(())
                        })
                        .unwrap();
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }

        assert_eq!(store.load().unwrap().profiles.len(), 8);
    }

    #[test]
    fn a_lock_held_too_long_times_out_naming_the_lock_file() {
        let store = ConfigStore::new(scratch("held"));
        fs::create_dir_all(store.dir()).unwrap();
        let _held = store.lock(Duration::from_secs(1)).unwrap();

        let error = store.lock(Duration::from_millis(200)).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        let lock_file = store.dir().join("config.lock");
        assert!(
            error.to_string().contains(&lock_file.display().to_string()),
            "{error}"
        );
    }

    #[test]
    fn a_queue_of_writers_that_keeps_moving_is_waited_for_past_the_timeout() {
        let store = ConfigStore::new(scratch("moving"));
        fs::create_dir_all(store.dir()).unwrap();
        let held = store.lock(TIMEOUT).unwrap();
        // Twenty writers, each taking a fifth of the timeout to finish: four times as long, all
        // told, as a holder that made no progress would be given.
        let mut queue = Waiting::queue(held, 20, TIMEOUT / 5);

        let taken = store.lock_with(TIMEOUT, &mut queue);

        assert!(taken.is_ok(), "{taken:?}");
        assert!(queue.elapsed() > TIMEOUT * 3, "{:?}", queue.elapsed());
    }

    #[test]
    fn a_holder_that_makes_no_progress_is_given_up_on_after_the_timeout() {
        let store = ConfigStore::new(scratch("stuck"));
        fs::create_dir_all(store.dir()).unwrap();
        let _held = store.lock(TIMEOUT).unwrap();
        let mut stuck = Waiting::stuck();

        let error = store.lock_with(TIMEOUT, &mut stuck).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        // The timeout is waited out, and not much more than it.
        assert!(
            (TIMEOUT..TIMEOUT + super::LOCK_POLL_MAX * 2).contains(&stuck.elapsed()),
            "{:?}",
            stuck.elapsed()
        );
    }

    #[test]
    fn a_write_to_the_configuration_is_what_counts_as_progress() {
        let store = ConfigStore::new(scratch("progress"));
        fs::create_dir_all(store.dir()).unwrap();
        let mut waiter = super::RealWaiter::new(store.file_path());

        let missing = waiter.progress();
        fs::write(store.file_path(), "# one").unwrap();
        let written = waiter.progress();
        fs::write(store.file_path(), "# one, then a longer second line").unwrap();
        let rewritten = waiter.progress();

        assert_eq!(missing, None);
        assert_ne!(written, missing);
        assert_ne!(rewritten, written);
    }

    #[test]
    fn lock_retries_are_spread_within_their_bounds() {
        let mut jitter = Jitter::new();
        let (low, high) = (Duration::from_millis(5), Duration::from_millis(10));
        let pauses: Vec<Duration> = (0..200).map(|_| jitter.between(low, high)).collect();

        assert!(pauses.iter().all(|pause| (low..=high).contains(pause)));
        assert!(pauses.iter().any(|pause| *pause != pauses[0]), "{pauses:?}");
    }

    #[test]
    fn a_broken_file_is_reported_with_its_path() {
        let store = ConfigStore::new(scratch("broken"));
        fs::create_dir_all(store.dir()).unwrap();
        fs::write(store.file_path(), "not [valid toml").unwrap();

        let error = store.load().unwrap_err();

        assert_eq!(error.exit.code(), 2);
        assert!(
            error.message.contains("config.toml is not valid TOML"),
            "{}",
            error.message
        );
    }
}
