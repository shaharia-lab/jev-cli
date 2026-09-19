//! Configuration: where it lives, how it is read, and how it is changed safely.

mod file;
mod paths;
mod settings;

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

pub(crate) use file::ConfigFile;
pub(crate) use paths::{CONFIG_FILE, config_dir};
pub(crate) use settings::{DEFAULT_PROFILE, Flags, Key, Setting, Settings, Source, Value};

use crate::error::CliError;

/// How long to wait for another `jev` process to finish writing before giving up.
const LOCK_TIMEOUT: Duration = Duration::from_secs(10);

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

        let _lock = self
            .lock()
            .map_err(|error| cannot_write("lock the configuration in", &error))?;
        let mut file = self.load()?;
        let outcome = change(&mut file)?;
        write_atomically(&self.file_path(), file.to_toml().as_bytes())
            .map_err(|error| cannot_write("write to", &error))?;
        Ok(outcome)
    }

    /// Takes the exclusive lock, which is released when the returned file is dropped. The lock is
    /// on a file of its own, because the configuration file itself is replaced on every write.
    fn lock(&self) -> io::Result<File> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.join("config.lock"))?;
        let started = Instant::now();
        loop {
            match lock.try_lock() {
                Ok(()) => return Ok(lock),
                Err(fs::TryLockError::WouldBlock) if started.elapsed() < LOCK_TIMEOUT => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(fs::TryLockError::WouldBlock) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "another jev process is holding the lock",
                    ));
                }
                Err(fs::TryLockError::Error(error)) => return Err(error),
            }
        }
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
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::{ConfigStore, Key, Value};

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
