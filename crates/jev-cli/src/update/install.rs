//! The installed binary, and the files the updater keeps beside it.
//!
//! Everything lives in `.jev-update/` next to the binary, so that every rename is within one file
//! system and therefore atomic:
//!
//! - `ready` (`ready.exe` on Windows) and `ready.version`: a verified binary waiting to be swapped
//!   in. `ready.version` is written first and the binary renamed into place last, so a binary
//!   that is there is always complete.
//! - `previous` (`previous.exe`): the binary before the last update, kept for `--rollback`.
//! - `receipt.json`: written by the install script, marking the install as self-managed.
//! - `trash/`: on Windows a running executable cannot be deleted or replaced, only renamed, so it
//!   is moved here and deleted by a later run.
//! - `update.lock`: held while one `jev` changes any of this.
//!
//! On Unix the binary is replaced with one `rename`, so there is always a complete `jev` at its
//! path. On Windows the running `jev.exe` is first renamed aside, then the new one renamed in; if
//! that fails, the old one is renamed back.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use semver::Version;

use crate::error::CliError;
use crate::exit::Exit;

/// The updater's directory, beside the binary.
const STATE_DIR: &str = ".jev-update";

/// How long to wait for the updater's lock before saying another `jev` holds it.
const LOCK_PATIENCE: Duration = Duration::from_secs(2);

/// How long the self-test of a new binary may take.
const SELF_TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// A `jev` binary on disk, and the updater's files beside it.
#[derive(Clone, Debug)]
pub(crate) struct Installation {
    exe: PathBuf,
    state: PathBuf,
}

impl Installation {
    /// The running `jev`.
    ///
    /// # Errors
    ///
    /// An internal error when the operating system cannot say where the running binary is.
    pub(crate) fn current() -> Result<Self, CliError> {
        let exe = std::env::current_exe().map_err(|error| {
            CliError::internal(format!("could not find the running jev binary: {error}"))
        })?;
        // On Unix links are resolved, so that the file replaced is the real binary and a Homebrew
        // link leads to its Cellar. Windows would return a `\\?\` path; its links are rare.
        let exe = if cfg!(unix) {
            fs::canonicalize(&exe).unwrap_or(exe)
        } else {
            exe
        };
        Ok(Self::new(exe))
    }

    /// The binary at `exe`.
    pub(crate) fn new(exe: PathBuf) -> Self {
        let state = exe
            .parent()
            .map_or_else(|| PathBuf::from(STATE_DIR), |dir| dir.join(STATE_DIR));
        Self { exe, state }
    }

    /// Where the binary is.
    pub(crate) fn exe(&self) -> &Path {
        &self.exe
    }

    /// The install script's receipt.
    pub(crate) fn receipt(&self) -> PathBuf {
        self.state.join("receipt.json")
    }

    /// The binary kept by the last update, for `--rollback`.
    pub(crate) fn previous(&self) -> PathBuf {
        self.state.join(binary("previous"))
    }

    fn ready(&self) -> PathBuf {
        self.state.join(binary("ready"))
    }

    fn ready_version(&self) -> PathBuf {
        self.state.join("ready.version")
    }

    fn trash(&self) -> PathBuf {
        self.state.join("trash")
    }

    /// A path in the updater's directory for a file being written, unique to this process.
    fn temporary(&self, purpose: &str) -> PathBuf {
        self.state.join(format!(
            "{purpose}.{}.tmp{}",
            std::process::id(),
            std::env::consts::EXE_SUFFIX
        ))
    }

    /// Creates the updater's directory and takes its lock, which is held until the returned
    /// value is dropped. Leftovers of earlier runs are cleared out.
    ///
    /// # Errors
    ///
    /// `update_not_writable` when the directory of the binary cannot be written to, and
    /// `update_in_progress` when another `jev` holds the lock.
    pub(crate) fn lock(&self) -> Result<Lock, CliError> {
        let not_writable = |error: &io::Error| {
            let dir = self.state.parent().unwrap_or(&self.state);
            CliError::update(
                "update_not_writable",
                Exit::Usage,
                format!("cannot write to {}: {error}", dir.display()),
            )
            .hint(
                "jev never asks for elevated rights. Reinstall it to a directory you can write to \
with the install script, or update it the way it was installed",
            )
        };
        fs::create_dir_all(self.trash()).map_err(|error| not_writable(&error))?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.state.join("update.lock"))
            .map_err(|error| not_writable(&error))?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => break,
                // A process being started elsewhere holds a copy of the descriptor for a moment,
                // and with it a Unix lock, so a busy lock is tried again for a little while.
                Err(fs::TryLockError::WouldBlock) if started.elapsed() < LOCK_PATIENCE => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(fs::TryLockError::WouldBlock) => {
                    return Err(CliError::update(
                        "update_in_progress",
                        Exit::Usage,
                        "another jev is updating this installation",
                    )
                    .hint("wait for it to finish, then run the command again"));
                }
                Err(fs::TryLockError::Error(error)) => return Err(not_writable(&error)),
            }
        }
        self.clear_leftovers();
        Ok(Lock(file))
    }

    /// Deletes what earlier runs left behind: renamed-aside binaries (which fail to delete while
    /// they still run) and temporary files of runs that were interrupted.
    fn clear_leftovers(&self) {
        if let Ok(entries) = fs::read_dir(self.trash()) {
            for entry in entries.flatten() {
                let _ = fs::remove_file(entry.path());
            }
        }
        if let Ok(entries) = fs::read_dir(&self.state) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().contains(".tmp") {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }

    /// Writes a verified binary to the staging directory as the one ready to be swapped in.
    ///
    /// # Errors
    ///
    /// `update_not_writable` when it cannot be written.
    pub(crate) fn stage(&self, binary: &[u8], version: &Version) -> Result<(), CliError> {
        let _lock = self.lock()?;
        let written = (|| {
            let _ = fs::remove_file(self.ready());
            write_new(
                &self.temporary("version"),
                version.to_string().as_bytes(),
                false,
            )?;
            fs::rename(self.temporary("version"), self.ready_version())?;
            write_new(&self.temporary("ready"), binary, true)?;
            fs::rename(self.temporary("ready"), self.ready())
        })();
        written.map_err(|error| {
            let _ = fs::remove_file(self.temporary("ready"));
            CliError::update(
                "update_not_writable",
                Exit::Usage,
                format!(
                    "could not stage the update in {}: {error}",
                    self.state.display()
                ),
            )
            .hint("check the free space and the permissions of that directory")
        })
    }

    /// The version of the binary that is staged and ready, if there is one.
    pub(crate) fn staged(&self) -> Option<Version> {
        if !self.ready().is_file() {
            return None;
        }
        let text = fs::read_to_string(self.ready_version()).ok()?;
        Version::parse(text.trim()).ok()
    }

    /// Removes a staged binary that will not be used, such as one no newer than the running `jev`
    /// or one that failed to go in. Best effort: what cannot be removed is tried again later.
    pub(crate) fn discard_staged(&self) {
        if let Ok(_lock) = self.lock() {
            let _ = fs::remove_file(self.ready());
            let _ = fs::remove_file(self.ready_version());
        }
    }

    /// Whether the binary can be replaced without elevated rights: its directory takes new files
    /// and the binary is not read-only. The file written to find out is removed at once.
    ///
    /// # Errors
    ///
    /// Why it cannot, for a person to read.
    pub(crate) fn writable(&self) -> Result<(), String> {
        let dir = self.exe.parent().unwrap_or_else(|| Path::new("."));
        let probe = dir.join(format!(".jev-write-test.{}", std::process::id()));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .map_err(|error| {
                format!(
                    "cannot write to {} ({error}), and jev never asks for elevated rights",
                    dir.display()
                )
            })?;
        let _ = fs::remove_file(&probe);
        if fs::metadata(&self.exe).is_ok_and(|metadata| metadata.permissions().readonly()) {
            return Err(format!(
                "{} is read-only, and jev never asks for elevated rights",
                self.exe.display()
            ));
        }
        Ok(())
    }

    /// Swaps the staged binary in, keeping the current one as the previous version, and runs the
    /// new one's self-test. When the self-test fails the current binary is put back.
    ///
    /// `self_test` runs the binary at the path it is given and says what went wrong.
    ///
    /// # Errors
    ///
    /// `update_not_staged` when no binary of `version` is staged, `update_not_writable` when a
    /// file cannot be moved, and `update_self_test_failed` when the new binary was rolled back.
    pub(crate) fn apply(
        &self,
        version: &Version,
        self_test: &dyn Fn(&Path) -> Result<(), String>,
    ) -> Result<(), CliError> {
        let _lock = self.lock()?;
        if self.staged().as_ref() != Some(version) {
            return Err(CliError::update(
                "update_not_staged",
                Exit::Internal,
                format!("no verified binary of jev {version} is staged"),
            )
            .hint("run `jev update` again"));
        }
        self.keep_current()
            .map_err(|error| self.move_failed(&error))?;
        self.replace(&self.ready(), &self.exe)
            .map_err(|error| self.move_failed(&error))?;
        let _ = fs::remove_file(self.ready_version());

        let Err(problem) = self_test(&self.exe) else {
            return Ok(());
        };
        let restored = self.copy_in(&self.previous(), "restore");
        let mut error = CliError::update(
            "update_self_test_failed",
            Exit::Internal,
            format!("the new jev {version} failed its self-test: {problem}"),
        );
        error = match restored {
            Ok(()) => error.hint("the previous binary was put back; nothing else changed"),
            Err(failure) => error.hint(format!(
                "putting the previous binary back failed too ({failure}): copy {} to {} by hand",
                self.previous().display(),
                self.exe.display()
            )),
        };
        Err(error)
    }

    /// Puts the previous binary back, keeping the current one as the previous version, so that a
    /// second rollback undoes the first.
    ///
    /// `probe` runs a binary and returns its version; the previous binary must pass it first.
    ///
    /// # Errors
    ///
    /// `no_previous_version` when no binary was kept, `update_self_test_failed` when the kept
    /// binary does not run, and `update_not_writable` when a file cannot be moved.
    pub(crate) fn rollback(
        &self,
        probe: &dyn Fn(&Path) -> Result<Version, String>,
    ) -> Result<Version, CliError> {
        let _lock = self.lock()?;
        if !self.previous().is_file() {
            return Err(CliError::update(
                "no_previous_version",
                Exit::Usage,
                "there is no previous version to roll back to",
            )
            .hint("a previous version is kept once `jev update` has replaced this binary; `jev update --version <x.y.z>` installs any release"));
        }
        let version = probe(&self.previous()).map_err(|problem| {
            CliError::update(
                "update_self_test_failed",
                Exit::Internal,
                format!("the previous binary does not run: {problem}"),
            )
            .hint("nothing was changed; `jev update --version <x.y.z>` installs a release instead")
        })?;
        let swap = self.temporary("swap");
        (|| {
            fs::copy(&self.exe, &swap)?;
            self.replace(&self.previous(), &self.exe)?;
            self.replace(&swap, &self.previous())
        })()
        .map_err(|error| self.move_failed(&error))?;
        Ok(version)
    }

    /// Copies the current binary to `previous`.
    fn keep_current(&self) -> io::Result<()> {
        let copy = self.temporary("previous");
        fs::copy(&self.exe, &copy)?;
        self.replace(&copy, &self.previous())
    }

    /// Copies `source` over the binary, leaving `source` where it is.
    fn copy_in(&self, source: &Path, purpose: &str) -> io::Result<()> {
        let copy = self.temporary(purpose);
        fs::copy(source, &copy)?;
        self.replace(&copy, &self.exe)
    }

    /// Renames `source` to `destination`, replacing it. Windows refuses to replace a running
    /// executable but lets it be renamed, so there the destination is moved aside first.
    fn replace(&self, source: &Path, destination: &Path) -> io::Result<()> {
        match fs::rename(source, destination) {
            Err(error) if cfg!(windows) && destination.exists() => {
                let aside = self.trash().join(format!(
                    "{}-{}{}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |elapsed| elapsed.as_nanos()),
                    std::env::consts::EXE_SUFFIX
                ));
                tracing::debug!(%error, "moving {} aside", destination.display());
                fs::rename(destination, &aside)?;
                fs::rename(source, destination).inspect_err(|_| {
                    let _ = fs::rename(&aside, destination);
                })
            }
            outcome => outcome,
        }
    }

    fn move_failed(&self, error: &io::Error) -> CliError {
        CliError::update(
            "update_not_writable",
            Exit::Usage,
            format!("could not replace {}: {error}", self.exe.display()),
        )
        .hint("check the permissions of that directory; jev never asks for elevated rights")
    }
}

/// The updater's lock; released when dropped.
#[derive(Debug)]
pub(crate) struct Lock(#[allow(dead_code)] File);

/// `stem` with the platform's executable extension, so that Windows will run it.
fn binary(stem: &str) -> String {
    format!("{stem}{}", std::env::consts::EXE_SUFFIX)
}

/// Writes a new file and flushes it to disk; an executable one is made runnable.
fn write_new(path: &Path, contents: &[u8], executable: bool) -> io::Result<()> {
    let _ = fs::remove_file(path);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if executable { 0o755 } else { 0o644 });
    }
    let _ = executable;
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

/// Runs `jev version` from the binary at `exe` and returns the version it reports.
///
/// This is the self-test of a new binary and the check that a kept one still runs. In a build
/// with the test hooks, `JEV_TEST_VERSION` is passed on as `expected` (or removed), because every
/// binary a test installs is the same build.
///
/// # Errors
///
/// What went wrong, for a person to read.
pub(crate) fn probe(exe: &Path, expected: Option<&Version>) -> Result<Version, String> {
    let mut command = Command::new(exe);
    command
        .args(["version", "--output", "json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(feature = "internal-test-hooks")]
    match expected {
        Some(version) => command.env("JEV_TEST_VERSION", version.to_string()),
        None => command.env_remove("JEV_TEST_VERSION"),
    };
    let mut child = command
        .spawn()
        .map_err(|error| format!("it could not be started: {error}"))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < SELF_TEST_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "`jev version` did not finish within {} seconds",
                    SELF_TEST_TIMEOUT.as_secs()
                ));
            }
            Err(error) => return Err(format!("it could not be waited for: {error}")),
        }
    };
    let mut stdout = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if !status.success() {
        return Err(format!("`jev version` failed ({status})"));
    }
    let reported = serde_json::from_str::<serde_json::Value>(&stdout)
        .ok()
        .and_then(|output| Version::parse(output.get("version")?.as_str()?).ok())
        .ok_or_else(|| "`jev version` did not report a version".to_owned())?;
    match expected {
        Some(version) if *version != reported => {
            Err(format!("it reports version {reported}, not {version}"))
        }
        _ => Ok(reported),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use semver::Version;

    use super::Installation;
    use crate::update::tests::scratch;

    /// An installation of a pretend binary holding `contents`.
    fn installed(contents: &str) -> Installation {
        let exe = scratch().join("bin").join(super::binary("jev"));
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, contents).unwrap();
        Installation::new(exe)
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    /// A self-test that passes.
    const PASSES: &dyn Fn(&Path) -> Result<(), String> = &|_| Ok(());

    #[test]
    fn a_staged_binary_is_swapped_in_and_the_old_one_kept() {
        let installation = installed("v1");
        let version = Version::new(2, 0, 0);

        assert_eq!(installation.staged(), None);
        installation.stage(b"v2", &version).unwrap();
        assert_eq!(installation.staged(), Some(version.clone()));
        installation.apply(&version, PASSES).unwrap();

        assert_eq!(read(installation.exe()), "v2");
        assert_eq!(read(&installation.previous()), "v1");
        assert_eq!(installation.staged(), None, "the staged binary was used up");
    }

    #[test]
    fn a_binary_that_fails_its_self_test_is_rolled_back() {
        let installation = installed("v1");
        let version = Version::new(2, 0, 0);
        installation.stage(b"broken", &version).unwrap();

        let error = installation
            .apply(&version, &|exe| {
                assert_eq!(read(exe), "broken", "the test runs the new binary");
                Err("it crashed".to_owned())
            })
            .unwrap_err();

        assert_eq!(error.code, "update_self_test_failed");
        assert!(error.message.contains("it crashed"), "{}", error.message);
        assert_eq!(read(installation.exe()), "v1");
        assert_eq!(read(&installation.previous()), "v1");
    }

    #[test]
    fn only_the_version_that_was_staged_is_applied() {
        let installation = installed("v1");
        installation.stage(b"v2", &Version::new(2, 0, 0)).unwrap();

        let error = installation
            .apply(&Version::new(3, 0, 0), PASSES)
            .unwrap_err();

        assert_eq!(error.code, "update_not_staged");
        assert_eq!(read(installation.exe()), "v1");
    }

    #[test]
    fn rollback_swaps_the_previous_binary_back_and_can_be_undone() {
        let installation = installed("v1");
        let version = Version::new(2, 0, 0);
        installation.stage(b"v2", &version).unwrap();
        installation.apply(&version, PASSES).unwrap();
        let probe = |exe: &Path| -> Result<Version, String> {
            Ok(if read(exe) == "v1" {
                Version::new(1, 0, 0)
            } else {
                Version::new(2, 0, 0)
            })
        };

        assert_eq!(
            installation.rollback(&probe).unwrap(),
            Version::new(1, 0, 0)
        );
        assert_eq!(read(installation.exe()), "v1");
        assert_eq!(read(&installation.previous()), "v2");

        assert_eq!(
            installation.rollback(&probe).unwrap(),
            Version::new(2, 0, 0)
        );
        assert_eq!(read(installation.exe()), "v2");
    }

    #[test]
    fn rollback_needs_a_previous_binary_that_runs() {
        let installation = installed("v1");
        let probe = |_: &Path| -> Result<Version, String> { Err("exec format error".to_owned()) };

        assert_eq!(
            installation.rollback(&probe).unwrap_err().code,
            "no_previous_version"
        );

        fs::write(installation.previous(), "garbage").unwrap();
        let error = installation.rollback(&probe).unwrap_err();
        assert_eq!(error.code, "update_self_test_failed");
        assert_eq!(read(installation.exe()), "v1", "nothing changed");
    }

    #[test]
    fn a_second_updater_waits_for_the_first() {
        let installation = installed("v1");
        let _held = installation.lock().unwrap();

        assert_eq!(
            installation
                .stage(b"v2", &Version::new(2, 0, 0))
                .unwrap_err()
                .code,
            "update_in_progress"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_that_cannot_be_written_is_reported_before_anything_changes() {
        use std::os::unix::fs::PermissionsExt;

        let installation = installed("v1");
        let dir = installation.exe().parent().unwrap().to_owned();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        let outcome = installation.stage(b"v2", &Version::new(2, 0, 0));
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

        // Root can write anywhere; the check only means something for everyone else.
        if let Err(error) = outcome {
            assert_eq!(error.code, "update_not_writable");
            assert!(
                error
                    .hint
                    .unwrap()
                    .contains("never asks for elevated rights")
            );
        }
        assert_eq!(read(installation.exe()), "v1");
    }

    #[test]
    fn a_staged_binary_can_be_discarded() {
        let installation = installed("v1");
        installation.stage(b"v2", &Version::new(2, 0, 0)).unwrap();

        installation.discard_staged();

        assert_eq!(installation.staged(), None);
        assert_eq!(read(installation.exe()), "v1");
    }

    #[test]
    fn a_binary_in_a_writable_directory_can_be_replaced_and_nothing_is_left_behind() {
        let installation = installed("v1");
        let dir = installation.exe().parent().unwrap().to_owned();

        assert_eq!(installation.writable(), Ok(()));
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");

        let mut permissions = fs::metadata(installation.exe()).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(installation.exe(), permissions.clone()).unwrap();
        let read_only = installation.writable();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(installation.exe(), permissions).unwrap();
        assert!(read_only.unwrap_err().contains("is read-only"));
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_that_cannot_be_written_means_the_binary_cannot_be_replaced() {
        use std::os::unix::fs::PermissionsExt;

        let installation = installed("v1");
        let dir = installation.exe().parent().unwrap().to_owned();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        let outcome = installation.writable();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

        // Root can write anywhere; the check only means something for everyone else.
        if let Err(reason) = outcome {
            assert!(
                reason.contains("never asks for elevated rights"),
                "{reason}"
            );
        }
    }

    #[test]
    fn the_self_test_of_something_that_is_not_jev_fails() {
        let installation = installed("not a program");

        assert!(super::probe(installation.exe(), None).is_err());
    }
}
