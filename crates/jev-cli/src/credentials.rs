//! Where the API key comes from.

use jev_client::ApiKey;

use crate::env::Env;
use crate::error::CliError;

/// The environment variable the official SDKs read, and so does `jev`.
pub(crate) const API_KEY_VARIABLE: &str = "TYPESAFE_API_KEY";

/// Something that can supply the API key for a profile.
///
/// Commands depend on this trait, so a test hands them a key without touching the environment or
/// a keychain. This build knows one source, the environment; stored credentials come with
/// `jev auth` (issue #10).
pub(crate) trait CredentialStore {
    /// The key for `profile`, and a word for where it came from.
    ///
    /// # Errors
    ///
    /// An authentication error (exit 3) when there is no key, or the one found cannot be a key.
    /// It names both remedies, and never contains the key.
    fn api_key(&self, profile: &str) -> Result<(ApiKey, &'static str), CliError>;
}

/// Reads the key from `TYPESAFE_API_KEY`.
pub(crate) struct EnvCredentials<'a>(pub(crate) &'a Env);

impl CredentialStore for EnvCredentials<'_> {
    fn api_key(&self, _profile: &str) -> Result<(ApiKey, &'static str), CliError> {
        let Some(key) = self.0.get(API_KEY_VARIABLE) else {
            return Err(
                CliError::auth("no_api_key", "no API key is configured").hint(format!(
                    "set {API_KEY_VARIABLE}, or store a key with `jev auth login`"
                )),
            );
        };
        ApiKey::new(key.to_owned())
            .map(|key| (key, "env"))
            .map_err(|error| {
                CliError::auth(
                    "invalid_api_key",
                    format!("{API_KEY_VARIABLE} does not hold a usable API key: {error}"),
                )
                .hint(format!(
                    "check the value of {API_KEY_VARIABLE}, or store a key with `jev auth login`"
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{CredentialStore, EnvCredentials};
    use crate::env::Env;

    #[test]
    fn reads_the_key_from_the_environment() {
        let env = Env::from([("TYPESAFE_API_KEY", "sentinel-key\n")]);

        let (key, source) = EnvCredentials(&env).api_key("default").unwrap();

        assert_eq!(key.expose(), "sentinel-key");
        assert_eq!(source, "env");
    }

    #[test]
    fn a_missing_or_unusable_key_exits_3_and_names_both_remedies_without_the_key() {
        let missing = EnvCredentials(&Env::default())
            .api_key("default")
            .unwrap_err();
        let unusable_env = Env::from([("TYPESAFE_API_KEY", "sentinel key with spaces")]);
        let unusable = EnvCredentials(&unusable_env)
            .api_key("default")
            .unwrap_err();

        for (error, code) in [(&missing, "no_api_key"), (&unusable, "invalid_api_key")] {
            assert_eq!(error.exit.code(), 3);
            assert_eq!(error.code, code);
            let hint = error.hint.as_deref().unwrap();
            assert!(
                hint.contains("TYPESAFE_API_KEY") && hint.contains("jev auth login"),
                "{hint}"
            );
            assert!(!format!("{error:?}").contains("sentinel"), "{error:?}");
        }
    }
}
