//! Automatic updates: a check after a command, in a process of its own, and the swap at the start
//! of the next command.
//!
//! When a command has finished, and at most once every [`INTERVAL`], `jev` starts itself again
//! as `jev update --background`, detached, with no standard streams, and exits without waiting for
//! it. That process asks for the newest release and, when it is an update, downloads, verifies and
//! stages it exactly as `jev update` does. At the start of the next command the staged binary is
//! swapped in, passes its self-test or is rolled back, and one line on stderr says so. The command
//! itself therefore never waits for the network, and its stdout and exit code are its own.
//!
//! The time of the last check, whether the first-run notice has been shown, and what the last check
//! found are kept in `auto-update.json` in the configuration directory. A check is claimed by
//! writing its time there under a lock that is never waited for, so of several `jev` processes
//! finishing together exactly one starts a check. A check that fails is not retried until the
//! next window. The background check itself has nothing left to do, so it is the one writer that
//! waits for the lock ([`RECORD_PATIENCE`]) rather than losing what it found.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use semver::Version;
use serde::{Deserialize, Serialize};

use super::{Installation, Newest, TrustedKeys, UpdateSource, newest, stage};
use crate::error::CliError;

/// The least time between two automatic checks.
pub(crate) const INTERVAL: Duration = Duration::from_hours(24);

/// The state of automatic updates, in the configuration directory.
const STATE_FILE: &str = "auto-update.json";

/// Held while `auto-update.json` is changed.
const LOCK_FILE: &str = "auto-update.lock";

/// How long [`StateFile::record`] waits for that lock. A command never waits for it, but the
/// background check is a process of its own with nothing left to do, and a lost outcome is one
/// `jev -v` can never show again.
const RECORD_PATIENCE: Duration = Duration::from_secs(10);

/// How long a waiting writer pauses between two attempts at the lock, which is only ever held for
/// the length of one small write.
const LOCK_POLL: Duration = Duration::from_millis(20);

/// What automatic updates remember between runs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct State {
    /// When the last check was started, in seconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_check: Option<u64>,
    /// Whether the notice that `jev` updates itself has been shown.
    #[serde(default)]
    pub(crate) notice_shown: bool,
    /// What the last check found, or why it failed. Shown with `-v`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_outcome: Option<String>,
}

impl State {
    /// Whether a check is due at `now`: none has been made, the last one is at least
    /// [`INTERVAL`] old, or the clock has been set back past it.
    pub(crate) fn due(&self, now: u64) -> bool {
        self.last_check
            .is_none_or(|last| last > now || now - last >= INTERVAL.as_secs())
    }
}

/// `auto-update.json` in a configuration directory.
#[derive(Clone, Debug)]
pub(crate) struct StateFile {
    dir: PathBuf,
}

impl StateFile {
    pub(crate) const fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// The state, or the state of a first run when the file is missing or unreadable.
    pub(crate) fn read(&self) -> State {
        fs::read(self.dir.join(STATE_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Changes the state while holding its lock. `change` returns `None` to write nothing.
    ///
    /// A caller waits `patience` for the lock, and [`Duration::ZERO`] is a caller that does not
    /// wait at all: when another `jev` holds the lock for longer than that, or the directory
    /// cannot be written, the result is `None` and nothing changes. The directory is created only
    /// when `create` is set, so that a background check finishing after its directory was removed
    /// does not bring it back.
    pub(crate) fn change<T>(
        &self,
        create: bool,
        patience: Duration,
        change: impl FnOnce(&mut State) -> Option<T>,
    ) -> Option<T> {
        if create {
            fs::create_dir_all(&self.dir).ok()?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.join(LOCK_FILE))
            .ok()?;
        let deadline = Instant::now() + patience;
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(LOCK_POLL);
                }
                Err(_) => return None,
            }
        }
        let mut state = self.read();
        let outcome = change(&mut state)?;
        let text = serde_json::to_vec(&state).ok()?;
        // Renamed into place, so that a reader never sees half of it. Not flushed to disk: this is
        // a cache that costs one extra check when it is lost, and a flush can stall for seconds on
        // a busy disk, which a command that must not wait cannot afford.
        let temporary = self
            .dir
            .join(format!(".{STATE_FILE}.{}.tmp", std::process::id()));
        let written = fs::write(&temporary, text)
            .and_then(|()| fs::rename(&temporary, self.dir.join(STATE_FILE)));
        if written.is_err() {
            let _ = fs::remove_file(&temporary);
            return None;
        }
        Some(outcome)
    }

    /// Claims the check that is due at `now`, if one is. Only one of several `jev` processes
    /// asking at once gets it.
    pub(crate) fn claim(&self, now: u64) -> bool {
        self.change(true, Duration::ZERO, |state| {
            state.due(now).then(|| {
                state.last_check = Some(now);
            })
        })
        .is_some()
    }

    /// Records the outcome of a check, waiting [`RECORD_PATIENCE`] for the lock.
    pub(crate) fn record(&self, outcome: String) {
        self.change(false, RECORD_PATIENCE, |state| {
            state.last_outcome = Some(outcome);
            Some(())
        });
    }
}

/// Seconds since the Unix epoch.
pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Starts `jev update --background` from the binary at `exe`, detached from this process and its
/// terminal, with no standard streams, and does not wait for it.
///
/// # Errors
///
/// When the process cannot be started.
pub(crate) fn spawn_check(exe: &Path) -> std::io::Result<()> {
    let mut command = Command::new(exe);
    command
        .args(["update", "--background"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // The check talks only to GitHub, and never with a key.
        .env_remove("TYPESAFE_API_KEY");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A group of its own, so that Ctrl-C in the terminal does not reach it.
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    // Not waited for: this process is about to exit, and the check must not hold it up.
    command.spawn().map(drop)
}

/// The background check: stages the newest release when it is an update from `current`.
///
/// Returns what it found, for `-v` in a later run.
///
/// # Errors
///
/// Whatever finding, downloading, verifying or staging the release fails with. The installed
/// binary is untouched in every case.
pub(crate) fn check(
    source: &dyn UpdateSource,
    keys: &TrustedKeys,
    installation: &Installation,
    current: &Version,
) -> Result<String, CliError> {
    Ok(match newest(source, current)? {
        Newest::NoRelease => "no release is published yet".to_owned(),
        Newest::UpToDate(release) => {
            format!("up to date (the latest release is {})", release.version)
        }
        Newest::Available(release) if installation.staged() == Some(release.version.clone()) => {
            format!("jev {} is already staged", release.version)
        }
        Newest::Available(release) => {
            stage(source, &release, keys, installation)?;
            format!("staged jev {}", release.version)
        }
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{INTERVAL, State, StateFile};
    use crate::update::tests::scratch;

    const DAY: u64 = INTERVAL.as_secs();

    #[test]
    fn a_check_is_due_once_a_day_and_when_the_clock_goes_back() {
        let now = 1_800_000_000;
        let at = |last_check| State {
            last_check,
            ..State::default()
        };

        assert!(at(None).due(now), "never checked");
        assert!(!at(Some(now)).due(now));
        assert!(!at(Some(now - DAY + 1)).due(now), "less than a day ago");
        assert!(at(Some(now - DAY)).due(now));
        assert!(at(Some(now + 60)).due(now), "the clock was set back");
    }

    #[test]
    fn only_the_first_claim_in_a_window_succeeds() {
        let file = StateFile::new(scratch().join("config"));
        let now = 1_800_000_000;

        assert!(file.claim(now), "the first run claims the check");
        assert!(!file.claim(now + 10), "within the window");
        assert!(!file.claim(now + DAY - 1));
        assert!(file.claim(now + DAY), "the next window");
        assert_eq!(file.read().last_check, Some(now + DAY));
    }

    #[test]
    fn concurrent_claims_start_exactly_one_check() {
        let file = StateFile::new(scratch().join("config"));
        let claims: Vec<_> = (0..16)
            .map(|_| {
                let file = file.clone();
                std::thread::spawn(move || file.claim(1_800_000_000))
            })
            .collect();

        // A thread that finds the lock taken claims nothing, like one that finds the check claimed.
        let granted = claims
            .into_iter()
            .map(|claim| claim.join().unwrap())
            .filter(|granted| *granted)
            .count();

        assert_eq!(granted, 1);
    }

    #[test]
    fn a_missing_or_damaged_file_is_a_first_run_and_is_replaced() {
        let dir = scratch().join("config");
        let file = StateFile::new(dir.clone());
        assert_eq!(file.read(), State::default());
        fs::create_dir_all(&dir).unwrap();

        fs::write(dir.join(super::STATE_FILE), "{ not json").unwrap();
        assert_eq!(file.read(), State::default());
        file.record("up to date".to_owned());

        let state = file.read();
        assert_eq!(state.last_outcome.as_deref(), Some("up to date"));
        assert!(!state.notice_shown);
    }

    #[test]
    fn an_outcome_is_recorded_even_when_another_jev_holds_the_lock_for_a_moment() {
        let dir = scratch().join("config");
        let file = StateFile::new(dir.clone());
        assert!(
            file.claim(1_800_000_000),
            "makes the directory and the lock"
        );
        let held = fs::OpenOptions::new()
            .write(true)
            .open(dir.join(super::LOCK_FILE))
            .unwrap();
        held.lock().unwrap();

        let recorder = {
            let file = file.clone();
            std::thread::spawn(move || file.record("staged jev 9.9.9".to_owned()))
        };
        // Held long enough that the record really does find the lock taken, and a hundredth of
        // the patience it waits for, so that a loaded machine cannot make this the other case.
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(held);
        recorder.join().unwrap();

        assert_eq!(
            file.read().last_outcome.as_deref(),
            Some("staged jev 9.9.9"),
            "a check that finds the lock taken waits for it"
        );
    }

    #[test]
    fn a_record_never_creates_the_directory() {
        let dir = scratch().join("removed");
        StateFile::new(dir.clone()).record("staged jev 9.9.9".to_owned());

        assert!(!dir.exists());
    }
}
