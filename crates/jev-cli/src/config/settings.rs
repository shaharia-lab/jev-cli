//! The settings every command reads, each with a record of where its value came from.

use std::fmt;
use std::time::Duration;

use clap::ValueEnum;
use indexmap::IndexMap;
use jev_client::BaseUrl;
use serde::Serialize;

use crate::duration;
use crate::env::Env;
use crate::error::CliError;
use crate::output::Format;
use crate::suggest;

/// The name of the profile that always exists.
pub(crate) const DEFAULT_PROFILE: &str = "default";

/// A setting that a profile can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    BaseUrl,
    Model,
    Output,
    Timeout,
    MaxRetries,
    Concurrency,
    WarnUnpinned,
    UpdateAuto,
    UpdateChannel,
    UpdatePinVersion,
}

impl Key {
    pub(crate) const ALL: [Self; 10] = [
        Self::BaseUrl,
        Self::Model,
        Self::Output,
        Self::Timeout,
        Self::MaxRetries,
        Self::Concurrency,
        Self::WarnUnpinned,
        Self::UpdateAuto,
        Self::UpdateChannel,
        Self::UpdatePinVersion,
    ];

    /// The table of `config.toml` that holds the setting once for every profile, or `None` for a
    /// setting each profile holds for itself. The updater's settings are about the binary, which
    /// every profile shares, so they live in `[update]`.
    pub(crate) const fn table(self) -> Option<&'static str> {
        match self {
            Self::UpdateAuto | Self::UpdateChannel | Self::UpdatePinVersion => Some("update"),
            _ => None,
        }
    }

    /// The name of the setting inside its table: `auto` for `update.auto`.
    pub(crate) fn field(self) -> &'static str {
        let name = self.name();
        name.rsplit_once('.').map_or(name, |(_, field)| field)
    }

    /// The name used in `config.toml` and on the command line.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::BaseUrl => "base_url",
            Self::Model => "model",
            Self::Output => "output",
            Self::Timeout => "timeout",
            Self::MaxRetries => "max_retries",
            Self::Concurrency => "concurrency",
            Self::WarnUnpinned => "warn_unpinned",
            Self::UpdateAuto => "update.auto",
            Self::UpdateChannel => "update.channel",
            Self::UpdatePinVersion => "update.pin_version",
        }
    }

    /// What the setting does, for `jev config list` and error hints.
    pub(crate) const fn describe(self) -> &'static str {
        match self {
            Self::BaseUrl => "API root",
            Self::Model => "model alias or versioned id",
            Self::Output => {
                "output format: table, json, yaml or jsonl (unset: table on a terminal, json when piped)"
            }
            Self::Timeout => "time allowed per attempt, e.g. 30s or 500ms",
            Self::MaxRetries => "retries after the first attempt",
            Self::Concurrency => "parallel requests in batch runs",
            Self::WarnUnpinned => {
                "remind me to pin a versioned model id when a request used an alias"
            }
            Self::UpdateAuto => "update jev automatically in the background",
            Self::UpdateChannel => "releases to update to: stable or prerelease",
            Self::UpdatePinVersion => {
                "stay on this version: turns automatic updates off (unset: follow the channel)"
            }
        }
    }

    /// The environment variable that overrides the setting, when there is one.
    pub(crate) const fn env_var(self) -> Option<&'static str> {
        match self {
            Self::BaseUrl => Some("TYPESAFE_BASE_URL"),
            Self::Model => Some("TYPESAFE_DEFAULT_MODEL"),
            Self::Output => Some("JEV_OUTPUT"),
            Self::UpdateAuto => Some("JEV_AUTO_UPDATE"),
            _ => None,
        }
    }

    /// The flag that overrides the setting, when there is one.
    pub(crate) const fn flag(self) -> Option<&'static str> {
        match self {
            Self::BaseUrl => Some("--base-url"),
            Self::Model => Some("--model"),
            Self::Output => Some("--output"),
            Self::Timeout => Some("--timeout"),
            Self::MaxRetries => Some("--max-retries"),
            Self::Concurrency => Some("--concurrency"),
            Self::WarnUnpinned
            | Self::UpdateAuto
            | Self::UpdateChannel
            | Self::UpdatePinVersion => None,
        }
    }

    /// The built-in default. `None` means "decided at run time" for `output`, and "none" for
    /// `update.pin_version`.
    pub(crate) fn default_value(self) -> Option<Value> {
        match self {
            Self::BaseUrl => Some(Value::Text("https://api.typesafe.ai".to_owned())),
            Self::Model => Some(Value::Text("jev-latest".to_owned())),
            Self::Output | Self::UpdatePinVersion => None,
            Self::Timeout => Some(Value::Duration(Duration::from_secs(30))),
            Self::MaxRetries => Some(Value::Count(2)),
            Self::Concurrency => Some(Value::Count(4)),
            Self::WarnUnpinned => Some(Value::Switch(false)),
            Self::UpdateAuto => Some(Value::Switch(true)),
            Self::UpdateChannel => Some(Value::Text("stable".to_owned())),
        }
    }

    /// Finds a key by name.
    ///
    /// # Errors
    ///
    /// A usage error listing the keys, with a suggestion for a near miss. A name that looks like a
    /// credential gets pointed at `jev auth login` instead: keys are never stored in the config.
    pub(crate) fn from_name(name: &str) -> Result<Self, CliError> {
        let normalised = name.trim().to_lowercase().replace('-', "_");
        if let Some(key) = Self::ALL.into_iter().find(|key| key.name() == normalised) {
            return Ok(key);
        }
        if ["key", "token", "secret", "password", "credential"]
            .iter()
            .any(|word| normalised.contains(word))
        {
            return Err(CliError::usage(format!(
                "`{name}` is not a setting: credentials are never stored in the config file"
            ))
            .hint("store the API key with `jev auth login`, or set TYPESAFE_API_KEY"));
        }
        let names = Self::ALL.map(Self::name);
        let error = CliError::usage(format!("`{name}` is not a setting"));
        Err(match suggest::closest(&normalised, names) {
            Some(known) => error.hint(format!(
                "did you mean `{known}`? The settings are: {}",
                names.join(", ")
            )),
            None => error.hint(format!("the settings are: {}", names.join(", "))),
        })
    }

    /// Reads a value written by a person, on the command line or in an environment variable.
    ///
    /// # Errors
    ///
    /// A usage error that says what the setting accepts.
    pub(crate) fn parse(self, text: &str) -> Result<Value, CliError> {
        let text = text.trim();
        let invalid = |problem: String| {
            CliError::usage(format!(
                "`{text}` is not a valid `{}`: {problem}",
                self.name()
            ))
            .hint(format!("`{}` is the {}", self.name(), self.describe()))
        };
        match self {
            // Only that it is a well-formed base URL. Whether plain http:// is acceptable is decided
            // where the connection is built, because that depends on --insecure-allow-http.
            Self::BaseUrl => BaseUrl::parse_allowing_insecure_http(text)
                .map(|_| Value::Text(text.to_owned()))
                .map_err(|error| invalid(error.to_string())),
            Self::Model if text.is_empty() => {
                Err(invalid("a model name cannot be empty".to_owned()))
            }
            Self::Model => Ok(Value::Text(text.to_owned())),
            Self::Output => <Format as ValueEnum>::from_str(text, true)
                .map(Value::Output)
                .map_err(|_| invalid("expected table, json, yaml or jsonl".to_owned())),
            Self::Timeout => duration::parse(text).map(Value::Duration).map_err(invalid),
            Self::MaxRetries => parse_count(text, 0, 100).map(Value::Count).map_err(invalid),
            Self::Concurrency => parse_count(text, 1, 64).map(Value::Count).map_err(invalid),
            Self::WarnUnpinned | Self::UpdateAuto => match text.to_lowercase().as_str() {
                "true" | "yes" | "on" | "1" => Ok(Value::Switch(true)),
                "false" | "no" | "off" | "0" => Ok(Value::Switch(false)),
                _ => Err(invalid("expected true or false".to_owned())),
            },
            Self::UpdateChannel => match text.to_lowercase().as_str() {
                channel @ ("stable" | "prerelease") => Ok(Value::Text(channel.to_owned())),
                _ => Err(invalid("expected stable or prerelease".to_owned())),
            },
            Self::UpdatePinVersion => semver::Version::parse(text.trim_start_matches('v'))
                .map(|version| Value::Text(version.to_string()))
                .map_err(|_| invalid("expected a version such as 0.3.1".to_owned())),
        }
    }
}

fn parse_count(text: &str, min: u32, max: u32) -> Result<u32, String> {
    text.parse::<u32>()
        .ok()
        .filter(|count| (min..=max).contains(count))
        .ok_or_else(|| format!("expected a whole number from {min} to {max}"))
}

/// The value of a setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    Text(String),
    Output(Format),
    Duration(Duration),
    Count(u32),
    Switch(bool),
}

impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => formatter.write_str(text),
            Self::Output(format) => formatter.write_str(format.name()),
            Self::Duration(duration) => formatter.write_str(&duration::format(*duration)),
            Self::Count(count) => write!(formatter, "{count}"),
            Self::Switch(on) => write!(formatter, "{on}"),
        }
    }
}

impl Serialize for Value {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Count(count) => serializer.serialize_u32(*count),
            Self::Switch(on) => serializer.serialize_bool(*on),
            text => serializer.collect_str(text),
        }
    }
}

/// Where a setting's value came from. Precedence is in this order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// A command-line flag.
    Flag(&'static str),
    /// An environment variable.
    Env(&'static str),
    /// A profile in `config.toml`.
    Profile(String),
    /// A table of `config.toml` outside the profiles, such as `[update]`; holds the setting's name.
    Config(&'static str),
    /// The built-in default.
    Default,
}

impl Source {
    /// `flag`, `env`, `profile` or `default`.
    pub(crate) const fn kind(&self) -> &'static str {
        match self {
            Self::Flag(_) => "flag",
            Self::Env(_) => "env",
            Self::Profile(_) => "profile",
            Self::Config(_) => "config",
            Self::Default => "default",
        }
    }

    /// The flag, the variable, the profile or the setting's name in the file; nothing for a
    /// default.
    pub(crate) fn origin(&self) -> Option<String> {
        match self {
            Self::Flag(flag) => Some((*flag).to_owned()),
            Self::Env(variable) => Some((*variable).to_owned()),
            Self::Profile(profile) => Some(profile.clone()),
            Self::Config(key) => Some((*key).to_owned()),
            Self::Default => None,
        }
    }
}

/// A setting's effective value and where it came from. `value` is `None` only for `output` when
/// nothing sets it, which means "text on a terminal, JSON on a pipe".
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Setting {
    pub(crate) value: Option<Value>,
    pub(crate) source: Source,
}

/// Values given on the command line.
#[derive(Clone, Debug, Default)]
pub(crate) struct Flags {
    pub(crate) profile: Option<String>,
    pub(crate) base_url: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) output: Option<Format>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) max_retries: Option<u32>,
    /// `jev batch run --concurrency`.
    pub(crate) concurrency: Option<u32>,
}

impl Flags {
    fn get(&self, key: Key) -> Option<Value> {
        match key {
            Key::BaseUrl => self.base_url.clone().map(Value::Text),
            Key::Model => self.model.clone().map(Value::Text),
            Key::Output => self.output.map(Value::Output),
            Key::Timeout => self.timeout.map(Value::Duration),
            Key::MaxRetries => self.max_retries.map(Value::Count),
            Key::Concurrency => self.concurrency.map(Value::Count),
            Key::WarnUnpinned | Key::UpdateAuto | Key::UpdateChannel | Key::UpdatePinVersion => {
                None
            }
        }
    }
}

/// The effective settings for one run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    /// The selected profile, and how it was selected.
    pub(crate) profile: Setting,
    values: IndexMap<Key, Setting>,
}

impl Settings {
    /// Resolves every setting: **flag, then environment, then the profile, then the default.**
    ///
    /// `active_profile`, `profiles` and `shared` (the settings held once for every profile, such
    /// as `[update]`) come from `config.toml`. The `default` profile always exists, whether or not
    /// the file mentions it.
    ///
    /// # Errors
    ///
    /// A usage error when the selected profile does not exist, or a flag or variable holds a value
    /// its setting does not accept. The message names the flag or the variable at fault.
    pub(crate) fn resolve(
        flags: &Flags,
        env: &Env,
        active_profile: Option<&str>,
        profiles: &IndexMap<String, IndexMap<Key, Value>>,
        shared: &IndexMap<Key, Value>,
    ) -> Result<Self, CliError> {
        let (name, source) = if let Some(name) = &flags.profile {
            (name.clone(), Source::Flag("--profile"))
        } else if let Some(name) = env.get("JEV_PROFILE") {
            (name.to_owned(), Source::Env("JEV_PROFILE"))
        } else if let Some(name) = active_profile {
            (
                name.to_owned(),
                Source::Profile("active_profile".to_owned()),
            )
        } else {
            (DEFAULT_PROFILE.to_owned(), Source::Default)
        };
        let stored = profiles.get(&name);
        if stored.is_none() && name != DEFAULT_PROFILE {
            let known: Vec<&str> = std::iter::once(DEFAULT_PROFILE)
                .chain(profiles.keys().map(String::as_str))
                .collect();
            let from = source.origin().unwrap_or_default();
            let error =
                CliError::usage(format!("there is no profile `{name}` (selected by {from})"));
            return Err(match suggest::closest(&name, known.iter().copied()) {
                Some(close) => error.hint(format!(
                    "did you mean `{close}`? Create a profile with `jev profile create {name}`"
                )),
                None => error.hint(format!(
                    "the profiles are: {}; create one with `jev profile create {name}`",
                    known.join(", ")
                )),
            });
        }

        let mut values = IndexMap::new();
        for key in Key::ALL {
            let setting = if let (Some(value), Some(flag)) = (flags.get(key), key.flag()) {
                // A flag was already validated by the argument parser, except for the base URL.
                let value = if key == Key::BaseUrl {
                    key.parse(&value.to_string())
                        .map_err(|error| blame(error, flag))?
                } else {
                    value
                };
                Setting {
                    value: Some(value),
                    source: Source::Flag(flag),
                }
            } else if let Some((variable, text)) = key
                .env_var()
                .and_then(|variable| Some((variable, env.get(variable)?)))
            {
                Setting {
                    value: Some(key.parse(text).map_err(|error| blame(error, variable))?),
                    source: Source::Env(variable),
                }
            } else if key.table().is_some() {
                shared.get(&key).map_or_else(
                    || Setting {
                        value: key.default_value(),
                        source: Source::Default,
                    },
                    |value| Setting {
                        value: Some(value.clone()),
                        source: Source::Config(key.name()),
                    },
                )
            } else if let Some(value) = stored.and_then(|profile| profile.get(&key)) {
                Setting {
                    value: Some(value.clone()),
                    source: Source::Profile(name.clone()),
                }
            } else {
                Setting {
                    value: key.default_value(),
                    source: Source::Default,
                }
            };
            values.insert(key, setting);
        }
        Ok(Self {
            profile: Setting {
                value: Some(Value::Text(name)),
                source,
            },
            values,
        })
    }

    /// One setting.
    pub(crate) fn get(&self, key: Key) -> &Setting {
        // `resolve` fills in every key, so the fallback is never reached.
        static MISSING: Setting = Setting {
            value: None,
            source: Source::Default,
        };
        self.values.get(&key).unwrap_or(&MISSING)
    }

    /// The name of the selected profile.
    pub(crate) fn profile_name(&self) -> &str {
        match &self.profile.value {
            Some(Value::Text(name)) => name,
            _ => DEFAULT_PROFILE,
        }
    }

    /// The value of an on-or-off setting.
    pub(crate) fn switch(&self, key: Key) -> bool {
        self.get(key).value == Some(Value::Switch(true))
    }

    /// The value of a setting that is text, when it has one.
    pub(crate) fn text(&self, key: Key) -> Option<&str> {
        match &self.get(key).value {
            Some(Value::Text(text)) => Some(text),
            _ => None,
        }
    }

    /// The output format, when a flag, a variable or the profile sets one.
    pub(crate) fn output(&self) -> Option<Format> {
        match self.get(Key::Output).value {
            Some(Value::Output(format)) => Some(format),
            _ => None,
        }
    }
}

/// Says which flag or variable held the bad value.
fn blame(mut error: CliError, origin: &str) -> CliError {
    error.message = format!("{origin}: {}", error.message);
    error
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use indexmap::IndexMap;

    use super::{DEFAULT_PROFILE, Flags, Key, Settings, Source, Value};
    use crate::env::Env;
    use crate::output::Format;

    type Profiles = IndexMap<String, IndexMap<Key, Value>>;

    fn profiles(entries: &[(&str, &[(Key, &str)])]) -> Profiles {
        entries
            .iter()
            .map(|(name, values)| {
                (
                    (*name).to_owned(),
                    values
                        .iter()
                        .map(|(key, text)| (*key, key.parse(text).unwrap()))
                        .collect(),
                )
            })
            .collect()
    }

    /// For every setting: a value for each layer that can set it, all of them different.
    fn layers(
        key: Key,
    ) -> (
        Option<Flags>,
        Option<(&'static str, &'static str)>,
        &'static str,
        &'static str,
    ) {
        match key {
            Key::BaseUrl => (
                Some(Flags {
                    base_url: Some("https://flag.example".into()),
                    ..Flags::default()
                }),
                Some(("TYPESAFE_BASE_URL", "https://env.example")),
                "https://profile.example",
                "https://api.typesafe.ai",
            ),
            Key::Model => (
                Some(Flags {
                    model: Some("flag-model".into()),
                    ..Flags::default()
                }),
                Some(("TYPESAFE_DEFAULT_MODEL", "env-model")),
                "profile-model",
                "jev-latest",
            ),
            Key::Output => (
                Some(Flags {
                    output: Some(Format::Yaml),
                    ..Flags::default()
                }),
                Some(("JEV_OUTPUT", "jsonl")),
                "table",
                "",
            ),
            Key::Timeout => (
                Some(Flags {
                    timeout: Some(Duration::from_secs(7)),
                    ..Flags::default()
                }),
                None,
                "9s",
                "30s",
            ),
            Key::MaxRetries => (
                Some(Flags {
                    max_retries: Some(7),
                    ..Flags::default()
                }),
                None,
                "9",
                "2",
            ),
            Key::Concurrency => (
                Some(Flags {
                    concurrency: Some(7),
                    ..Flags::default()
                }),
                None,
                "9",
                "4",
            ),
            Key::WarnUnpinned => (None, None, "true", "false"),
            Key::UpdateAuto => (None, Some(("JEV_AUTO_UPDATE", "false")), "false", "true"),
            Key::UpdateChannel => (None, None, "prerelease", "stable"),
            Key::UpdatePinVersion => (None, None, "0.3.1", ""),
        }
    }

    type Stored<'a> = Vec<(Key, &'a str)>;

    /// Resolves with `values` stored where `config.toml` keeps each key: in the default profile,
    /// or in the table shared by every profile.
    fn resolve_stored(flags: &Flags, env: &Env, values: &[(Key, &str)]) -> Settings {
        let (shared, own): (Stored<'_>, Stored<'_>) =
            values.iter().partition(|(key, _)| key.table().is_some());
        let shared = shared
            .into_iter()
            .map(|(key, text)| (key, key.parse(text).unwrap()))
            .collect();
        Settings::resolve(
            flags,
            env,
            None,
            &profiles(&[(DEFAULT_PROFILE, own.as_slice())]),
            &shared,
        )
        .unwrap()
    }

    /// Where a value stored in `config.toml` is reported to come from.
    fn stored_source(key: Key) -> Source {
        if key.table().is_some() {
            Source::Config(key.name())
        } else {
            Source::Profile(DEFAULT_PROFILE.into())
        }
    }

    #[test]
    fn every_setting_follows_flag_then_env_then_profile_then_default() {
        for key in Key::ALL {
            let (flags, variable, profile_value, default_value) = layers(key);
            let stored = [(key, profile_value)];
            let env = variable.map_or_else(Env::default, |pair| Env::from([pair]));
            let shown = |settings: &Settings| {
                settings
                    .get(key)
                    .value
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default()
            };

            // Everything set: the highest layer that exists for this setting wins.
            let all = resolve_stored(&flags.clone().unwrap_or_default(), &env, &stored);
            match (&flags, variable) {
                (Some(_), _) => assert!(
                    matches!(all.get(key).source, Source::Flag(_)),
                    "{key:?}: {:?}",
                    all.get(key)
                ),
                (None, Some(_)) => {
                    assert!(matches!(all.get(key).source, Source::Env(_)), "{key:?}");
                }
                (None, None) => assert_eq!(all.get(key).source, stored_source(key), "{key:?}"),
            }

            // Without the flag: the environment, when this setting has a variable.
            let no_flag = resolve_stored(&Flags::default(), &env, &stored);
            if let Some((name, value)) = variable {
                assert_eq!(no_flag.get(key).source, Source::Env(name), "{key:?}");
                assert_eq!(shown(&no_flag), value, "{key:?}");
            }

            // Without flag or environment: the profile, or the table every profile shares.
            let profile_only = resolve_stored(&Flags::default(), &Env::default(), &stored);
            assert_eq!(profile_only.get(key).source, stored_source(key), "{key:?}");
            assert_eq!(shown(&profile_only), profile_value, "{key:?}");

            // With nothing set: the default.
            let nothing = resolve_stored(&Flags::default(), &Env::default(), &[]);
            assert_eq!(nothing.get(key).source, Source::Default, "{key:?}");
            assert_eq!(shown(&nothing), default_value, "{key:?}");
        }
    }

    #[test]
    fn the_profile_is_selected_by_flag_then_env_then_the_active_profile() {
        let stored = profiles(&[
            ("work", &[(Key::Model, "work-model")]),
            ("home", &[]),
            ("lab", &[]),
        ]);
        let env = Env::from([("JEV_PROFILE", "home")]);
        let flag = Flags {
            profile: Some("work".into()),
            ..Flags::default()
        };

        let by_flag =
            Settings::resolve(&flag, &env, Some("lab"), &stored, &IndexMap::new()).unwrap();
        let by_env = Settings::resolve(
            &Flags::default(),
            &env,
            Some("lab"),
            &stored,
            &IndexMap::new(),
        )
        .unwrap();
        let by_config = Settings::resolve(
            &Flags::default(),
            &Env::default(),
            Some("lab"),
            &stored,
            &IndexMap::new(),
        )
        .unwrap();
        let by_default = Settings::resolve(
            &Flags::default(),
            &Env::default(),
            None,
            &stored,
            &IndexMap::new(),
        )
        .unwrap();

        assert_eq!(
            (by_flag.profile_name(), by_flag.profile.source.kind()),
            ("work", "flag")
        );
        assert_eq!(
            by_flag.get(Key::Model).source,
            Source::Profile("work".into())
        );
        assert_eq!(
            (by_env.profile_name(), by_env.profile.source.kind()),
            ("home", "env")
        );
        assert_eq!(
            (by_config.profile_name(), by_config.profile.source.kind()),
            ("lab", "profile")
        );
        assert_eq!(
            (by_default.profile_name(), by_default.profile.source.kind()),
            ("default", "default")
        );
    }

    #[test]
    fn an_unknown_profile_is_an_error_but_default_always_exists() {
        let stored = profiles(&[("staging", &[])]);
        let typo = Flags {
            profile: Some("stagin".into()),
            ..Flags::default()
        };

        let error =
            Settings::resolve(&typo, &Env::default(), None, &stored, &IndexMap::new()).unwrap_err();

        assert_eq!(error.exit.code(), 2);
        assert_eq!(
            error.message,
            "there is no profile `stagin` (selected by --profile)"
        );
        assert!(error.hint.unwrap().starts_with("did you mean `staging`?"));
        assert!(
            Settings::resolve(
                &Flags::default(),
                &Env::default(),
                Some("default"),
                &Profiles::new(),
                &IndexMap::new()
            )
            .is_ok()
        );
    }

    #[test]
    fn a_bad_value_names_the_variable_or_flag_at_fault() {
        let env = Env::from([("JEV_OUTPUT", "xml")]);
        let flags = Flags {
            base_url: Some("ftp://example.com".into()),
            ..Flags::default()
        };

        let from_env = Settings::resolve(
            &Flags::default(),
            &env,
            None,
            &Profiles::new(),
            &IndexMap::new(),
        )
        .unwrap_err();
        let from_flag = Settings::resolve(
            &flags,
            &Env::default(),
            None,
            &Profiles::new(),
            &IndexMap::new(),
        )
        .unwrap_err();

        assert!(
            from_env
                .message
                .starts_with("JEV_OUTPUT: `xml` is not a valid `output`"),
            "{}",
            from_env.message
        );
        assert!(
            from_flag.message.starts_with("--base-url: "),
            "{}",
            from_flag.message
        );
        assert!(
            from_flag.message.contains("https://"),
            "{}",
            from_flag.message
        );
    }

    #[test]
    fn values_are_validated_per_setting() {
        assert!(Key::Timeout.parse("500ms").is_ok());
        assert!(Key::WarnUnpinned.parse("off").is_ok());
        for (text, on) in [
            ("false", false),
            ("0", false),
            ("no", false),
            ("OFF", false),
            ("on", true),
        ] {
            assert_eq!(
                Key::UpdateAuto.parse(text).unwrap(),
                Value::Switch(on),
                "{text}"
            );
        }
        assert_eq!(
            Key::UpdatePinVersion.parse("v0.3.1").unwrap(),
            Value::Text("0.3.1".into())
        );
        assert_eq!(
            Key::UpdateChannel.parse("PreRelease").unwrap(),
            Value::Text("prerelease".into())
        );
        assert!(Key::BaseUrl.parse("http://127.0.0.1:4010").is_ok());
        for (key, wrong) in [
            (Key::BaseUrl, "ftp://example.com"),
            (Key::BaseUrl, "https://user:pw@example.com"),
            (Key::Model, " "),
            (Key::Output, "xml"),
            (Key::Timeout, "0"),
            (Key::MaxRetries, "-1"),
            (Key::MaxRetries, "101"),
            (Key::Concurrency, "0"),
            (Key::Concurrency, "65"),
            (Key::WarnUnpinned, "maybe"),
            (Key::UpdateAuto, "sometimes"),
            (Key::UpdateChannel, "nightly"),
            (Key::UpdatePinVersion, "latest"),
        ] {
            let error = key.parse(wrong).unwrap_err();
            assert_eq!(error.exit.code(), 2, "{key:?} {wrong:?}");
            assert!(error.hint.is_some());
        }
    }

    #[test]
    fn unknown_setting_names_get_a_suggestion_and_credentials_get_redirected() {
        assert_eq!(Key::from_name("Base-URL").unwrap(), Key::BaseUrl);

        let typo = Key::from_name("modle").unwrap_err();
        assert!(typo.hint.unwrap().starts_with("did you mean `model`?"));

        for name in ["api_key", "TYPESAFE_API_KEY", "token", "secret"] {
            let error = Key::from_name(name).unwrap_err();
            assert!(
                error.message.contains("credentials are never stored"),
                "{name}: {}",
                error.message
            );
            assert!(error.hint.unwrap().contains("jev auth login"));
        }
    }

    #[test]
    fn values_serialise_with_their_natural_json_type() {
        let json = |key: Key, text: &str| serde_json::to_value(key.parse(text).unwrap()).unwrap();

        assert_eq!(json(Key::MaxRetries, "3"), 3);
        assert_eq!(json(Key::WarnUnpinned, "no"), false);
        assert_eq!(json(Key::Timeout, "1.5"), "1500ms");
        assert_eq!(json(Key::Output, "JSON"), "json");
    }
}
