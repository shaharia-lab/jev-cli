//! Where the API key comes from, and where `jev auth login` puts it.
//!
//! The key is looked for in `TYPESAFE_API_KEY`, then in the operating system's keychain, then in
//! the `credentials` file. It is never accepted as a flag value, never written to `config.toml`,
//! and nothing here ever formats it: errors from the stores are described by kind, because some of
//! them carry the secret they failed on.

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

/// The name under which `jev` keeps its entries in the keychain.
const KEYCHAIN_SERVICE: &str = "jev-cli";

/// The name of the fallback file, next to `config.toml`.
const CREDENTIALS_FILE: &str = "credentials";

/// Where a key was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeySource {
    Env,
    Keychain,
    File,
}

impl KeySource {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Env => "env",
            Self::Keychain => "keychain",
            Self::File => "file",
        }
    }
}

/// Something that can supply the API key for a profile.
///
/// Commands depend on this trait, so a test hands them a key without touching the environment, a
/// keychain or a file.
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

/// A place a key can be kept in: the keychain, or the credentials file.
pub(crate) trait SecretStore {
    fn source(&self) -> KeySource;

    fn get(&self, profile: &str) -> Result<Option<ApiKey>, CliError>;

    fn set(&self, profile: &str, key: &ApiKey) -> Result<(), CliError>;

    /// Removes the key. Returns whether there was one.
    fn delete(&self, profile: &str) -> Result<bool, CliError>;
}

/// The stores of this machine, in the order they are consulted.
pub(crate) struct Credentials<'a> {
    env: &'a Env,
    /// `None` when there is no usable keychain, or `JEV_NO_KEYCHAIN` is set.
    pub(crate) keychain: Option<Box<dyn SecretStore>>,
    pub(crate) file: FileStore,
}

impl<'a> Credentials<'a> {
    pub(crate) fn new(env: &'a Env, config_dir: &Path) -> Self {
        let disabled = env.get("JEV_NO_KEYCHAIN").is_some_and(|value| {
            !matches!(value.to_ascii_lowercase().as_str(), "0" | "false" | "no")
        });
        let keychain: Option<Box<dyn SecretStore>> = if disabled || !Keychain::is_available() {
            None
        } else {
            Some(Box::new(Keychain))
        };
        Self {
            env,
            keychain,
            file: FileStore::new(config_dir),
        }
    }

    /// The stores a key can be saved in, the keychain first.
    pub(crate) fn stores(&self) -> impl Iterator<Item = &dyn SecretStore> {
        self.keychain
            .iter()
            .map(AsRef::as_ref)
            .chain(std::iter::once(&self.file as &dyn SecretStore))
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
        for store in self.stores() {
            if let Some(key) = store.get(profile)? {
                return Ok(Some((key, store.source())));
            }
        }
        Ok(None)
    }
}

/// The operating system's keychain: macOS Keychain, Windows Credential Manager, or the Secret
/// Service on Linux.
pub(crate) struct Keychain;

impl Keychain {
    /// Whether this machine has a keychain `jev` can talk to. A headless Linux box usually has none.
    fn is_available() -> bool {
        keyring::Entry::store_status().is_ok()
    }

    fn entry(profile: &str) -> Result<keyring::Entry, CliError> {
        keyring::Entry::new(KEYCHAIN_SERVICE, &format!("profile:{profile}"))
            .map_err(|error| keychain_error("opened", &error))
    }
}

impl SecretStore for Keychain {
    fn source(&self) -> KeySource {
        KeySource::Keychain
    }

    fn get(&self, profile: &str) -> Result<Option<ApiKey>, CliError> {
        match Self::entry(profile)?.get_password() {
            Ok(key) => ApiKey::new(key).map(Some).map_err(|error| {
                CliError::auth(
                    "invalid_api_key",
                    format!(
                        "the key in the keychain for profile `{profile}` is not usable: {error}"
                    ),
                )
                .hint("store it again with `jev auth login`")
            }),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keychain_error("read", &error)),
        }
    }

    fn set(&self, profile: &str, key: &ApiKey) -> Result<(), CliError> {
        Self::entry(profile)?
            .set_password(key.expose())
            .map_err(|error| keychain_error("written to", &error))
    }

    fn delete(&self, profile: &str) -> Result<bool, CliError> {
        match Self::entry(profile)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(error) => Err(keychain_error("changed", &error)),
        }
    }
}

/// Describes a keychain failure by kind. The error is never formatted as it is: two of its
/// variants carry the bytes of the secret they could not decode.
fn keychain_error(action: &str, error: &keyring::Error) -> CliError {
    let reason = match error {
        keyring::Error::NoStorageAccess(platform) | keyring::Error::PlatformFailure(platform) => {
            platform.to_string()
        }
        keyring::Error::BadEncoding(_) | keyring::Error::BadDataFormat(..) => {
            "the stored value is not text".to_owned()
        }
        keyring::Error::NoEntry => "there is no entry".to_owned(),
        _ => "the keychain refused".to_owned(),
    };
    CliError::auth("keychain_unavailable", format!("the keychain could not be {action}: {reason}"))
        .hint(format!("unlock the keychain and try again; or set {API_KEY_VARIABLE}; or use the credentials file with `jev auth login --insecure-storage` (JEV_NO_KEYCHAIN=1 skips the keychain)"))
}

/// The `credentials` file: readable by its owner only, in a directory only its owner can enter.
///
/// It exists for machines without a keychain. The key is stored in clear text, which is why
/// `jev auth login` asks first, or says so when it cannot ask.
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

impl SecretStore for FileStore {
    fn source(&self) -> KeySource {
        KeySource::File
    }

    fn get(&self, profile: &str) -> Result<Option<ApiKey>, CliError> {
        let document = self.load()?;
        let Some(key) = document
            .get("profiles")
            .and_then(|profiles| profiles.get(profile))
            .and_then(|entry| entry.get("api_key"))
            .and_then(Item::as_str)
        else {
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

    fn set(&self, profile: &str, key: &ApiKey) -> Result<(), CliError> {
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

    fn delete(&self, profile: &str) -> Result<bool, CliError> {
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
pub(crate) mod testing {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use jev_client::ApiKey;

    use super::{KeySource, SecretStore};
    use crate::error::CliError;

    /// A keychain that lives in memory.
    #[derive(Default)]
    pub(crate) struct FakeKeychain(pub(crate) RefCell<HashMap<String, String>>);

    impl SecretStore for FakeKeychain {
        fn source(&self) -> KeySource {
            KeySource::Keychain
        }

        fn get(&self, profile: &str) -> Result<Option<ApiKey>, CliError> {
            Ok(self
                .0
                .borrow()
                .get(profile)
                .map(|key| ApiKey::new(key.clone()).unwrap()))
        }

        fn set(&self, profile: &str, key: &ApiKey) -> Result<(), CliError> {
            self.0
                .borrow_mut()
                .insert(profile.to_owned(), key.expose().to_owned());
            Ok(())
        }

        fn delete(&self, profile: &str) -> Result<bool, CliError> {
            Ok(self.0.borrow_mut().remove(profile).is_some())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use jev_client::ApiKey;

    use super::testing::FakeKeychain;
    use super::{CredentialStore, Credentials, FileStore, KeySource, SecretStore, fingerprint};
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

    fn credentials<'a>(env: &'a Env, dir: &std::path::Path, keychain: bool) -> Credentials<'a> {
        let keychain: Option<Box<dyn SecretStore>> = if keychain {
            Some(Box::new(FakeKeychain::default()))
        } else {
            None
        };
        Credentials {
            env,
            keychain,
            file: FileStore::new(dir),
        }
    }

    #[test]
    fn the_environment_wins_then_the_keychain_then_the_file() {
        let dir = scratch("order");
        let with_env = Env::from([("TYPESAFE_API_KEY", "key-from-the-environment")]);
        let without_env = Env::default();
        let stores = credentials(&without_env, &dir, true);
        stores
            .file
            .set("default", &key("key-from-the-file"))
            .unwrap();

        assert_eq!(stores.find("default").unwrap().unwrap().1, KeySource::File);
        stores
            .keychain
            .as_ref()
            .unwrap()
            .set("default", &key("key-from-the-keychain"))
            .unwrap();
        let (found, source) = stores.find("default").unwrap().unwrap();
        assert_eq!(
            (found.expose(), source),
            ("key-from-the-keychain", KeySource::Keychain)
        );

        let mut overridden = credentials(&with_env, &dir, false);
        overridden.keychain = stores.keychain;
        let (found, source) = overridden.find("default").unwrap().unwrap();
        assert_eq!(
            (found.expose(), source),
            ("key-from-the-environment", KeySource::Env)
        );
    }

    #[test]
    fn keys_are_kept_per_profile_and_can_be_removed() {
        let dir = scratch("profiles");
        let env = Env::default();
        let stores = credentials(&env, &dir, true);

        for store in stores.stores() {
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
    }

    #[test]
    fn no_key_anywhere_exits_3_and_names_every_remedy() {
        let dir = scratch("none");
        let env = Env::default();

        let error = credentials(&env, &dir, true)
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
