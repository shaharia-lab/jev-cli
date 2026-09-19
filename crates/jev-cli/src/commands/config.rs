//! `jev config get|set|unset|list|path`.

use serde::Serialize;

use super::Context;
use crate::cli::ConfigCommand;
use crate::config::{Key, Setting, Settings, Source, Value};
use crate::error::CliError;
use crate::notice::Notice;
use crate::output::{Cell, Render, Table, Ui};

pub(crate) fn run(command: &ConfigCommand, context: &mut Context<'_>) -> Result<(), CliError> {
    match command {
        ConfigCommand::Get { key } => {
            let key = Key::from_name(key)?;
            let row = Row::new(key, context.settings()?.get(key));
            context.output.emit(&row, context.stdout)
        }
        ConfigCommand::Set { key, value } => {
            let key = Key::from_name(key)?;
            let value = key.parse(value)?;
            let profile = context.settings()?.profile_name().to_owned();
            if key == Key::BaseUrl && jev_client::BaseUrl::parse(&value.to_string()).is_err() {
                context.notify(
                    &Notice::warning("insecure_http", "this base URL is plain http:// to a host that is not localhost")
                        .hint("every command using it will need --insecure-allow-http, and will send the API key unencrypted"),
                );
            }
            context.store()?.update(|file| {
                file.set(&profile, key, &value);
                Ok(())
            })?;
            let changed = Changed {
                profile,
                key: key.name(),
                value: Some(value),
                applied: true,
            };
            context.output.emit(&changed, context.stdout)
        }
        ConfigCommand::Unset { key } => {
            let key = Key::from_name(key)?;
            let profile = context.settings()?.profile_name().to_owned();
            let removed = context
                .store()?
                .update(|file| Ok(file.unset(&profile, key)))?;
            let changed = Changed {
                profile,
                key: key.name(),
                value: None,
                applied: removed,
            };
            context.output.emit(&changed, context.stdout)
        }
        ConfigCommand::List => {
            let listing = Listing::new(
                context.settings()?,
                context.store()?.file_path().display().to_string(),
            );
            context.output.emit(&listing, context.stdout)
        }
        ConfigCommand::Path => {
            let store = context.store()?;
            let path = Location {
                config_dir: store.dir().display().to_string(),
                config_file: store.file_path().display().to_string(),
                exists: store.file_path().is_file(),
            };
            context.output.emit(&path, context.stdout)
        }
    }
}

/// One setting: its effective value, and where that value comes from.
#[derive(Debug, Serialize)]
struct Row {
    key: &'static str,
    /// `null` only for `output` when nothing sets it: text on a terminal, JSON on a pipe.
    value: Option<Value>,
    /// `flag`, `env`, `profile` or `default`.
    source: &'static str,
    /// The flag, the environment variable or the profile that supplied the value.
    origin: Option<String>,
    /// What the setting does.
    description: &'static str,
}

impl Row {
    fn new(key: Key, setting: &Setting) -> Self {
        Self {
            key: key.name(),
            value: setting.value.clone(),
            source: setting.source.kind(),
            origin: setting.source.origin(),
            description: key.describe(),
        }
    }

    fn shown_value(&self) -> String {
        self.value
            .as_ref()
            .map_or_else(|| "(auto)".to_owned(), ToString::to_string)
    }

    fn shown_source(&self) -> String {
        match &self.origin {
            Some(origin) if self.source == "profile" => format!("profile `{origin}`"),
            Some(origin) if self.source == "config" => format!("config {origin}"),
            Some(origin) => format!("{} {origin}", self.source),
            None => self.source.to_owned(),
        }
    }
}

impl Render for Row {
    /// Just the value, so that `$(jev config get model)` works in a script.
    fn human(&self, _: Ui) -> String {
        format!("{}\n", self.shown_value())
    }
}

/// Every setting, as `jev config list` reports them.
#[derive(Debug, Serialize)]
struct Listing {
    /// The selected profile, and what selected it.
    profile: Row,
    settings: Vec<Row>,
    config_file: String,
}

impl Listing {
    fn new(settings: &Settings, config_file: String) -> Self {
        // A profile cannot come "from a profile": when the file selects it, say so.
        let (source, origin) = match &settings.profile.source {
            Source::Profile(_) => ("config", Some("active_profile".to_owned())),
            other => (other.kind(), other.origin()),
        };
        let profile = Row {
            key: "profile",
            value: settings.profile.value.clone(),
            source,
            origin,
            description: "the selected profile",
        };
        Self {
            profile,
            settings: Key::ALL
                .into_iter()
                .map(|key| Row::new(key, settings.get(key)))
                .collect(),
            config_file,
        }
    }
}

impl Render for Listing {
    fn human(&self, ui: Ui) -> String {
        let mut table = Table::default();
        table.row(
            ["SETTING", "VALUE", "SOURCE"]
                .map(|title| Cell::styled(title, ui.dim(title)))
                .into(),
        );
        for row in std::iter::once(&self.profile).chain(&self.settings) {
            let value = row.shown_value();
            table.row(vec![
                Cell::plain(row.key),
                Cell::styled(&value, ui.bold(&value)),
                Cell::plain(row.shown_source()),
            ]);
        }
        format!(
            "{}{}\n",
            table.render(""),
            ui.dim(&format!("config file: {}", self.config_file))
        )
    }

    fn records(&self) -> Option<Vec<serde_json::Value>> {
        let rows = std::iter::once(&self.profile).chain(&self.settings);
        Some(
            rows.filter_map(|row| serde_json::to_value(row).ok())
                .collect(),
        )
    }
}

/// The outcome of `set` and `unset`.
#[derive(Debug, Serialize)]
struct Changed {
    profile: String,
    key: &'static str,
    /// The stored value; `null` after `unset`.
    value: Option<Value>,
    /// `false` when `unset` found nothing to remove.
    #[serde(rename = "changed")]
    applied: bool,
}

impl Render for Changed {
    fn human(&self, _: Ui) -> String {
        match (&self.value, self.applied) {
            (Some(value), _) => format!(
                "set `{}` to `{value}` in profile `{}`\n",
                self.key, self.profile
            ),
            (None, true) => format!("removed `{}` from profile `{}`\n", self.key, self.profile),
            (None, false) => format!("`{}` was not set in profile `{}`\n", self.key, self.profile),
        }
    }
}

/// Where the configuration lives.
#[derive(Debug, Serialize)]
struct Location {
    config_dir: String,
    config_file: String,
    exists: bool,
}

impl Render for Location {
    fn human(&self, _: Ui) -> String {
        format!("{}\n", self.config_file)
    }
}
