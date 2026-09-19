//! `jev profile list|use|create|delete`.

use indexmap::IndexMap;
use serde::Serialize;

use super::Context;
use crate::cli::ProfileCommand;
use crate::config::{ConfigFile, DEFAULT_PROFILE, Flags, Key, Value};
use crate::error::CliError;
use crate::output::{Cell, Render, Table, Ui};
use crate::suggest;

pub(crate) fn run(command: &ProfileCommand, context: &mut Context<'_>) -> Result<(), CliError> {
    match command {
        ProfileCommand::List => {
            let file = context.config_file()?;
            let selected = context.settings()?.profile_name().to_owned();
            let active = file
                .active_profile
                .clone()
                .unwrap_or_else(|| DEFAULT_PROFILE.to_owned());
            let profiles = file
                .profile_names()
                .into_iter()
                .map(|name| Profile {
                    name: name.to_owned(),
                    active: name == active,
                    selected: name == selected,
                    settings: file.profiles.get(name).map(named).unwrap_or_default(),
                })
                .collect();
            context.output.emit(&Profiles { profiles }, context.stdout)
        }
        ProfileCommand::Use { name } => {
            context.store()?.update(|file| {
                ensure_exists(file, name)?;
                file.set_active_profile(name);
                Ok(())
            })?;
            context.output.emit(
                &Outcome {
                    profile: name.clone(),
                    action: "active",
                },
                context.stdout,
            )
        }
        ProfileCommand::Create { name } => {
            validate_name(name)?;
            let initial = initial_settings(&context.configuration.flags)?;
            context.store()?.update(|file| {
                if file.has_profile(name) {
                    return Err(
                        CliError::usage(format!("profile `{name}` already exists")).hint(format!(
                            "change it with `jev config set <key> <value> --profile {name}`"
                        )),
                    );
                }
                file.create_profile(name);
                for (key, value) in &initial {
                    file.set(name, *key, value);
                }
                Ok(())
            })?;
            context.output.emit(
                &Outcome {
                    profile: name.clone(),
                    action: "created",
                },
                context.stdout,
            )
        }
        ProfileCommand::Delete { name } => {
            context.store()?.update(|file| {
                ensure_exists(file, name)?;
                if !file.delete_profile(name) {
                    return Err(CliError::usage(format!(
                        "profile `{name}` has no stored settings to delete"
                    ))
                    .hint("the `default` profile always exists; there is nothing to remove"));
                }
                Ok(())
            })?;
            context.output.emit(
                &Outcome {
                    profile: name.clone(),
                    action: "deleted",
                },
                context.stdout,
            )
        }
    }
}

fn ensure_exists(file: &ConfigFile, name: &str) -> Result<(), CliError> {
    if file.has_profile(name) {
        return Ok(());
    }
    let names = file.profile_names();
    let error = CliError::usage(format!("there is no profile `{name}`"));
    Err(match suggest::closest(name, names.iter().copied()) {
        Some(close) => error.hint(format!("did you mean `{close}`?")),
        None => error.hint(format!(
            "the profiles are: {}; create one with `jev profile create {name}`",
            names.join(", ")
        )),
    })
}

/// A profile name is used in a TOML key, a flag value and (later) a keychain entry, so it is kept
/// to characters that need no quoting anywhere.
fn validate_name(name: &str) -> Result<(), CliError> {
    let valid = (1..=64).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && name
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphanumeric());
    if valid {
        return Ok(());
    }
    Err(
        CliError::usage(format!("`{name}` is not a valid profile name"))
            .hint("use 1 to 64 letters, digits, `-` or `_`, starting with a letter or a digit"),
    )
}

/// The settings a new profile starts with: whatever was passed as a global flag.
fn initial_settings(flags: &Flags) -> Result<Vec<(Key, Value)>, CliError> {
    let mut settings = Vec::new();
    if let Some(base_url) = &flags.base_url {
        settings.push((Key::BaseUrl, Key::BaseUrl.parse(base_url)?));
    }
    if let Some(model) = &flags.model {
        settings.push((Key::Model, Key::Model.parse(model)?));
    }
    if let Some(output) = flags.output {
        settings.push((Key::Output, Value::Output(output)));
    }
    if let Some(timeout) = flags.timeout {
        settings.push((Key::Timeout, Value::Duration(timeout)));
    }
    if let Some(max_retries) = flags.max_retries {
        settings.push((Key::MaxRetries, Value::Count(max_retries)));
    }
    Ok(settings)
}

fn named(values: &IndexMap<Key, Value>) -> IndexMap<&'static str, Value> {
    values
        .iter()
        .map(|(key, value)| (key.name(), value.clone()))
        .collect()
}

#[derive(Debug, Serialize)]
struct Profiles {
    profiles: Vec<Profile>,
}

#[derive(Debug, Serialize)]
struct Profile {
    name: String,
    /// The profile used when neither --profile nor `JEV_PROFILE` selects another.
    active: bool,
    /// The profile this very command ran with.
    selected: bool,
    /// The settings stored in the profile. Anything absent uses its default.
    settings: IndexMap<&'static str, Value>,
}

impl Render for Profiles {
    fn human(&self, ui: Ui) -> String {
        let mut table = Table::default();
        table.row(
            ["", "PROFILE", "SETTINGS"]
                .map(|title| Cell::styled(title, ui.dim(title)))
                .into(),
        );
        for profile in &self.profiles {
            let settings: Vec<String> = profile
                .settings
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect();
            let settings = if settings.is_empty() {
                "(defaults)".to_owned()
            } else {
                settings.join("  ")
            };
            table.row(vec![
                Cell::plain(if profile.active { "*" } else { "" }),
                Cell::styled(&profile.name, ui.bold(&profile.name)),
                Cell::plain(settings),
            ]);
        }
        table.render("")
    }

    fn records(&self) -> Option<Vec<serde_json::Value>> {
        Some(
            self.profiles
                .iter()
                .filter_map(|profile| serde_json::to_value(profile).ok())
                .collect(),
        )
    }
}

#[derive(Debug, Serialize)]
struct Outcome {
    profile: String,
    /// `created`, `deleted` or `active`.
    action: &'static str,
}

impl Render for Outcome {
    fn human(&self, _: Ui) -> String {
        match self.action {
            "active" => format!("profile `{}` is now active\n", self.profile),
            action => format!("{action} profile `{}`\n", self.profile),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::validate_name;

    #[test]
    fn profile_names_need_no_quoting_anywhere() {
        for name in ["work", "staging-2", "team_a", "9lives", &"a".repeat(64)] {
            assert!(validate_name(name).is_ok(), "{name}");
        }
        for name in [
            "",
            "-work",
            "_x",
            "has space",
            "dot.ted",
            "slash/ed",
            "üni",
            &"a".repeat(65),
        ] {
            let error = validate_name(name).unwrap_err();
            assert_eq!(error.exit.code(), 2, "{name:?}");
        }
    }
}
