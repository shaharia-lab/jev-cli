//! Where the API key comes from, and where `jev auth login` puts it.
//!
//! There are exactly two places: the `TYPESAFE_API_KEY` environment variable, which always wins,
//! and the `credentials` file that `jev auth login` writes. TypeSafe has one way to authenticate a
//! developer, a bearer API key, so there is nothing else to support, and deliberately no keychain:
//! one rule that is the same on every machine beats a nicer store that behaves differently on each.
//!
//! The key is never accepted as a flag value, never written to `config.toml`, and nothing here
//! ever formats it. A parse error of the credentials file is not shown either, because it would
//! quote a line that holds a key.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use jev_client::ApiKey;
use toml_edit::{DocumentMut, Item, Table, value};
use zeroize::Zeroizing;

use crate::env::Env;
use crate::error::CliError;

/// The environment variable the official SDKs read, and so does `jev`.
pub(crate) const API_KEY_VARIABLE: &str = "TYPESAFE_API_KEY";

/// Where API keys are created.
pub(crate) const KEY_CONSOLE_URL: &str = "https://console.typesafe.ai/keys";

/// The name of the file, next to `config.toml`.
const CREDENTIALS_FILE: &str = "credentials";

/// Where a key was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeySource {
    Env,
    File,
}

impl KeySource {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Env => "env",
            Self::File => "file",
        }
    }
}

/// Something that can supply the API key for a profile.
///
/// Commands depend on this trait, so a test hands them a key without touching the environment or
/// a file.
pub(crate) trait CredentialStore {
    /// The key for `profile` and where it came from, or `None` when there is none anywhere.
    ///
    /// # Errors
    ///
    /// An authentication error (exit 3) when a key was found but cannot be used, or a store could
    /// not be read. It never contains the key.
    fn find(&self, profile: &str) -> Result<Option<(ApiKey, KeySource)>, CliError>;

    /// The key for `profile`.
    ///
    /// # Errors
    ///
    /// An authentication error (exit 3) naming every remedy when there is no key.
    fn api_key(&self, profile: &str) -> Result<(ApiKey, KeySource), CliError> {
        self.find(profile)?.ok_or_else(|| {
            CliError::auth("no_api_key", format!("no API key is configured for profile `{profile}`")).hint(format!(
                "create a key at {KEY_CONSOLE_URL}, then either `export {API_KEY_VARIABLE}=...` or store it with `jev auth login`"
            ))
        })
    }
}

/// The key sources of this machine: the environment, then the credentials file.
pub(crate) struct Credentials<'a> {
    env: &'a Env,
    pub(crate) file: FileStore,
}

impl<'a> Credentials<'a> {
    pub(crate) fn new(env: &'a Env, config_dir: &Path) -> Self {
        Self {
            env,
            file: FileStore::new(config_dir),
        }
    }
}

impl CredentialStore for Credentials<'_> {
    fn find(&self, profile: &str) -> Result<Option<(ApiKey, KeySource)>, CliError> {
        if let Some(key) = self.env.get(API_KEY_VARIABLE) {
            return ApiKey::new(key.to_owned()).map(|key| Some((key, KeySource::Env))).map_err(|error| {
                CliError::auth("invalid_api_key", format!("{API_KEY_VARIABLE} does not hold a usable API key: {error}"))
                    .hint(format!("check the value of {API_KEY_VARIABLE}; keys are created at {KEY_CONSOLE_URL}"))
            });
        }
        Ok(self.file.get(profile)?.map(|key| (key, KeySource::File)))
    }
}

/// The `credentials` file: readable by its owner only, in a directory only its owner can enter.
///
/// The key is stored in clear text, like `~/.aws/credentials`. What protects it is the file's
/// mode: it is private from the first byte written, and `jev` refuses to read it once anyone else
/// can.
#[derive(Clone, Debug)]
pub(crate) struct FileStore {
    path: PathBuf,
}

impl FileStore {
    pub(crate) fn new(config_dir: &Path) -> Self {
        Self {
            path: config_dir.join(CREDENTIALS_FILE),
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the file. A file that does not exist is an empty one.
    fn load(&self) -> Result<DocumentMut, CliError> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => Zeroizing::new(text),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(DocumentMut::new()),
            Err(error) => return Err(self.unreadable(&error.to_string())),
        };
        self.check_permissions()?;
        // The parse error is not shown: it would quote a line of a file that holds keys.
        text.parse::<DocumentMut>()
            .map_err(|_| self.unreadable("it is not valid TOML"))
    }

    fn unreadable(&self, why: &str) -> CliError {
        CliError::auth(
            "credentials_unreadable",
            format!("cannot read {}: {why}", self.path.display()),
        )
        .hint(format!(
            "fix or delete the file, then run `jev auth login`; or set {API_KEY_VARIABLE}"
        ))
    }

    /// Refuses a file that anyone but its owner can read.
    #[cfg(unix)]
    fn check_permissions(&self) -> Result<(), CliError> {
        use std::os::unix::fs::PermissionsExt;

        let mode = fs::metadata(&self.path)
            .map_err(|error| self.unreadable(&error.to_string()))?
            .permissions()
            .mode()
            & 0o777;
        // A permission mask reads best as a mask: no bit for group or others may be set.
        #[allow(clippy::verbose_bit_mask)]
        let private = mode & 0o077 == 0;
        if private {
            return Ok(());
        }
        Err(CliError::auth(
            "credentials_too_open",
            format!(
                "{} can be read by other users (its mode is {mode:03o}), so jev will not use it",
                self.path.display()
            ),
        )
        .hint(format!("run `chmod 600 {}`", self.path.display())))
    }

    /// Windows keeps a user's profile private through its ACLs; there is no mode to check.
    #[cfg(not(unix))]
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    fn check_permissions(&self) -> Result<(), CliError> {
        Ok(())
    }

    /// Replaces the file: a private temporary file, then a rename, so that it is never seen half
    /// written and never, even briefly, readable by anyone else.
    fn save(&self, document: &DocumentMut) -> Result<(), CliError> {
        let cannot_write = |error: &io::Error| {
            CliError::auth(
                "credentials_unwritable",
                format!("cannot write {}: {error}", self.path.display()),
            )
            .hint(format!(
                "check the directory's permissions; or set {API_KEY_VARIABLE} instead"
            ))
        };
        let dir = self.path.parent().unwrap_or_else(|| Path::new("."));
        create_private_dir(dir).map_err(|error| cannot_write(&error))?;

        let temporary = self
            .path
            .with_file_name(format!(".{CREDENTIALS_FILE}.{}.tmp", std::process::id()));
        let content = Zeroizing::new(document.to_string());
        let written = (|| {
            let mut file = private_file_options()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)
        })();
        if written.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        written.map_err(|error| cannot_write(&error))
    }
}

/// Options for a file only its owner can read, from the moment it is created.
#[cfg(unix)]
fn private_file_options() -> OpenOptions {
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = OpenOptions::new();
    options.mode(0o600);
    options
}

/// On Windows a file inherits the ACLs of the user's profile, which already keep others out.
#[cfg(not(unix))]
fn private_file_options() -> OpenOptions {
    OpenOptions::new()
}

/// Creates the directory, and makes sure only its owner can enter it.
fn create_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

const FILE_HEADER: &str = "# jev credentials. This file holds API keys in clear text: keep it private (mode 0600).\n\
# Manage it with `jev auth login` and `jev auth logout`.\n";

impl FileStore {
    /// The key stored for `profile`, if there is one.
    pub(crate) fn get(&self, profile: &str) -> Result<Option<ApiKey>, CliError> {
        let document = self.load()?;
        let Some(key) = stored_key(&document, profile) else {
            return Ok(None);
        };
        ApiKey::new(key.to_owned()).map(Some).map_err(|error| {
            CliError::auth(
                "invalid_api_key",
                format!(
                    "the key in {} for profile `{profile}` is not usable: {error}",
                    self.path.display()
                ),
            )
            .hint("store it again with `jev auth login`")
        })
    }

    /// Stores the key for `profile`, replacing any it already has.
    pub(crate) fn set(&self, profile: &str, key: &ApiKey) -> Result<(), CliError> {
        let mut document = self.load()?;
        if document.is_empty() {
            document.decor_mut().set_prefix(FILE_HEADER);
        }
        let profiles = document.entry("profiles").or_insert_with(|| {
            let mut table = Table::new();
            table.set_implicit(true);
            Item::Table(table)
        });
        if let Some(profiles) = profiles.as_table_mut() {
            let entry = profiles
                .entry(profile)
                .or_insert_with(|| Item::Table(Table::new()));
            if let Some(entry) = entry.as_table_mut() {
                entry.insert("api_key", value(key.expose()));
            }
        }
        self.save(&document)
    }

    /// Removes the key of `profile`. Returns whether there was one.
    pub(crate) fn delete(&self, profile: &str) -> Result<bool, CliError> {
        if !self.path.exists() {
            return Ok(false);
        }
        let mut document = self.load()?;
        let removed = document
            .get_mut("profiles")
            .and_then(Item::as_table_mut)
            .is_some_and(|profiles| profiles.remove(profile).is_some());
        if removed {
            self.save(&document)?;
        }
        Ok(removed)
    }
}

/// The key a credentials file holds for `profile`, as written.
pub(crate) fn stored_key<'a>(document: &'a DocumentMut, profile: &str) -> Option<&'a str> {
    document
        .get("profiles")
        .and_then(|profiles| profiles.get(profile))
        .and_then(|entry| entry.get("api_key"))
        .and_then(Item::as_str)
}

/// The last four characters of a key, which is all of it that is ever shown.
pub(crate) fn fingerprint(key: &ApiKey) -> String {
    let characters: Vec<char> = key.expose().chars().collect();
    // A key too short to have a meaningful tail shows nothing at all.
    if characters.len() < 12 {
        return "…".to_owned();
    }
    format!(
        "…{}",
        characters
            .iter()
            .skip(characters.len() - 4)
            .collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use jev_client::ApiKey;

    use super::{CredentialStore, Credentials, FileStore, KeySource, fingerprint};
    use crate::env::Env;

    const SENTINEL: &str = "sentinel-key-0123456789-wxyz";

    fn scratch(name: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "jev-credentials-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn key(text: &str) -> ApiKey {
        ApiKey::new(text.to_owned()).unwrap()
    }

    #[test]
    fn the_environment_wins_over_the_file() {
        let dir = scratch("order");
        let with_env = Env::from([("TYPESAFE_API_KEY", "key-from-the-environment")]);
        let without_env = Env::default();
        Credentials::new(&without_env, &dir)
            .file
            .set("default", &key("key-from-the-file"))
            .unwrap();

        let (found, source) = Credentials::new(&without_env, &dir)
            .find("default")
            .unwrap()
            .unwrap();
        assert_eq!(
            (found.expose(), source),
            ("key-from-the-file", KeySource::File)
        );

        let (found, source) = Credentials::new(&with_env, &dir)
            .find("default")
            .unwrap()
            .unwrap();
        assert_eq!(
            (found.expose(), source),
            ("key-from-the-environment", KeySource::Env)
        );
    }

    #[test]
    fn an_unusable_key_in_the_environment_is_an_error_not_a_fall_through_to_the_file() {
        let dir = scratch("unusable-env");
        let env = Env::from([("TYPESAFE_API_KEY", "sentinel key with spaces")]);
        Credentials::new(&Env::default(), &dir)
            .file
            .set("default", &key("key-from-the-file"))
            .unwrap();

        let error = Credentials::new(&env, &dir).find("default").unwrap_err();

        assert_eq!((error.exit.code(), error.code), (3, "invalid_api_key"));
        assert!(!format!("{error:?}").contains("sentinel key"), "{error:?}");
    }

    #[test]
    fn keys_are_kept_per_profile_and_can_be_removed() {
        let store = FileStore::new(&scratch("profiles"));

        store.set("work", &key("the-work-profile-key")).unwrap();
        assert_eq!(
            store.get("work").unwrap().unwrap().expose(),
            "the-work-profile-key"
        );
        assert!(store.get("default").unwrap().is_none());
        assert!(store.delete("work").unwrap());
        assert!(!store.delete("work").unwrap(), "already gone");
        assert!(store.get("work").unwrap().is_none());
    }

    #[test]
    fn no_key_anywhere_exits_3_and_names_every_remedy() {
        let env = Env::default();

        let error = Credentials::new(&env, &scratch("none"))
            .api_key("staging")
            .unwrap_err();

        assert_eq!(error.exit.code(), 3);
        assert_eq!(
            error.message,
            "no API key is configured for profile `staging`"
        );
        let hint = error.hint.unwrap();
        for remedy in [
            "TYPESAFE_API_KEY",
            "jev auth login",
            "https://console.typesafe.ai/keys",
        ] {
            assert!(hint.contains(remedy), "{hint}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_private_from_the_moment_it_exists_and_so_is_its_directory() {
        use std::os::unix::fs::PermissionsExt;

        let dir = scratch("modes").join("jev");
        let store = FileStore::new(&dir);

        store.set("default", &key(SENTINEL)).unwrap();

        assert_eq!(
            fs::metadata(store.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let text = fs::read_to_string(store.path()).unwrap();
        assert!(text.starts_with("# jev credentials."), "{text}");
        assert!(text.contains("[profiles.default]\napi_key = "), "{text}");
    }

    #[cfg(unix)]
    #[test]
    fn a_file_others_can_read_is_refused_with_the_exact_chmod() {
        use std::os::unix::fs::PermissionsExt;

        let dir = scratch("too-open");
        let store = FileStore::new(&dir);
        store.set("default", &key(SENTINEL)).unwrap();
        fs::set_permissions(store.path(), fs::Permissions::from_mode(0o644)).unwrap();

        let error = store.get("default").unwrap_err();

        assert_eq!(error.exit.code(), 3);
        assert!(
            error.message.contains("its mode is 644"),
            "{}",
            error.message
        );
        assert_eq!(
            error.hint.as_deref(),
            Some(format!("run `chmod 600 {}`", store.path().display()).as_str())
        );
        assert!(!format!("{error:?}").contains(SENTINEL));
    }

    #[test]
    fn a_damaged_file_is_reported_without_quoting_it() {
        let dir = scratch("damaged");
        let store = FileStore::new(&dir);
        store.set("default", &key(SENTINEL)).unwrap();
        let broken = format!("[profiles.default\napi_key = \"{SENTINEL}\"");
        fs::write(store.path(), broken).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(store.path(), fs::Permissions::from_mode(0o600)).unwrap();
        }

        let error = store.get("default").unwrap_err();

        assert!(
            error.message.ends_with("it is not valid TOML"),
            "{}",
            error.message
        );
        assert!(!format!("{error:?}").contains(SENTINEL), "{error:?}");
    }

    #[test]
    fn only_the_last_four_characters_of_a_key_are_ever_shown() {
        assert_eq!(fingerprint(&key(SENTINEL)), "…wxyz");
        assert_eq!(
            fingerprint(&key("short-key")),
            "…",
            "a short key shows nothing"
        );
    }
}
