//! `config.toml`: reading it tolerantly, and changing it without disturbing what a person wrote.

use indexmap::IndexMap;
use toml_edit::{DocumentMut, Item, Table, value};

use super::settings::{DEFAULT_PROFILE, Key, Value};
use crate::duration;
use crate::error::CliError;
use crate::notice::Notice;
use crate::suggest;

/// The version of the file layout this build writes and fully understands.
pub(crate) const SCHEMA_VERSION: i64 = 1;

/// What a new file starts with.
const HEADER: &str = "# jev configuration. Change it with `jev config set` and `jev profile`, or by hand.\n\
# API keys are never stored in this file.\n";

/// Top-level keys. `[update]` holds the settings every profile shares (see [`Key::table`]).
const TOP_LEVEL_KEYS: [&str; 5] = [
    "schema_version",
    "active_profile",
    "profiles",
    "update",
    "pricing",
];

/// A parsed `config.toml`.
///
/// Reading is tolerant: an unknown key is a warning, never an error, so a file written by a newer
/// `jev` still works. A value of the wrong type *is* an error, because silently ignoring
/// `timeout = "soon"` would mean silently using a different timeout.
#[derive(Clone, Debug)]
pub(crate) struct ConfigFile {
    document: DocumentMut,
    pub(crate) active_profile: Option<String>,
    pub(crate) profiles: IndexMap<String, IndexMap<Key, Value>>,
    /// The settings held once for every profile, such as `[update] auto`.
    pub(crate) shared: IndexMap<Key, Value>,
    /// `[pricing] usd_per_mtok`: the user's own rate, used instead of the built-in price list.
    pub(crate) pricing_usd_per_mtok: Option<f64>,
    pub(crate) warnings: Vec<Notice>,
}

impl Default for ConfigFile {
    fn default() -> Self {
        Self {
            document: DocumentMut::new(),
            active_profile: None,
            profiles: IndexMap::new(),
            shared: IndexMap::new(),
            pricing_usd_per_mtok: None,
            warnings: Vec::new(),
        }
    }
}

impl ConfigFile {
    /// Parses the text of a `config.toml`. `shown_path` is only used in messages.
    ///
    /// # Errors
    ///
    /// A usage error when the text is not TOML, or a known key holds a value it cannot have.
    pub(crate) fn parse(text: &str, shown_path: &str) -> Result<Self, CliError> {
        let document: DocumentMut = text.parse().map_err(|error: toml_edit::TomlError| {
            CliError::usage(format!(
                "{shown_path} is not valid TOML: {}",
                first_line(&error.to_string())
            ))
            .hint("fix the file by hand, or move it aside and let jev create a new one")
        })?;
        let invalid = |key: &str, problem: &str| {
            CliError::usage(format!("{shown_path}: `{key}` {problem}"))
                .hint("fix the value by hand, or remove the line")
        };

        let mut warnings = Vec::new();
        for (key, _) in document
            .iter()
            .filter(|(key, _)| !TOP_LEVEL_KEYS.contains(key))
        {
            warnings.push(unknown_key(shown_path, key, key, &TOP_LEVEL_KEYS));
        }
        match document.get("schema_version").map(Item::as_integer) {
            None | Some(Some(..=SCHEMA_VERSION)) => {}
            Some(Some(newer)) => warnings.push(
                Notice::warning(
                    "config_newer_schema",
                    format!("{shown_path} was written by a newer jev (schema_version {newer}; this build knows {SCHEMA_VERSION})"),
                )
                .hint("run `jev update`; settings this build does not know are ignored"),
            ),
            Some(None) => return Err(invalid("schema_version", "must be a whole number")),
        }
        let active_profile = match document.get("active_profile") {
            None => None,
            Some(item) => Some(
                item.as_str()
                    .ok_or_else(|| invalid("active_profile", "must be a profile name"))?
                    .to_owned(),
            ),
        };

        let mut profiles = IndexMap::new();
        if let Some(item) = document.get("profiles") {
            let tables = item
                .as_table_like()
                .ok_or_else(|| invalid("profiles", "must be a table of profiles"))?;
            for (name, profile) in tables.iter() {
                let path = format!("profiles.{name}");
                let entries = profile
                    .as_table_like()
                    .ok_or_else(|| invalid(&path, "must be a table of settings"))?;
                let mut values = IndexMap::new();
                for (key_name, item) in entries.iter() {
                    let key_path = format!("{path}.{key_name}");
                    match profile_keys().find(|key| key.name() == key_name) {
                        Some(key) => {
                            let parsed = from_toml(key, item)
                                .map_err(|problem| invalid(&key_path, &problem))?;
                            values.insert(key, parsed);
                        }
                        None => warnings.push(unknown_key(
                            shown_path,
                            &key_path,
                            key_name,
                            &profile_keys().map(Key::name).collect::<Vec<_>>(),
                        )),
                    }
                }
                profiles.insert(name.to_owned(), values);
            }
        }
        let shared = read_shared(&document, shown_path, &invalid, &mut warnings)?;
        let pricing_usd_per_mtok = match document
            .get("pricing")
            .and_then(|pricing| pricing.get("usd_per_mtok"))
        {
            None => None,
            Some(item) => {
                let rate = item.as_float().or_else(|| {
                    item.as_integer()
                        .and_then(|whole| i32::try_from(whole).ok())
                        .map(f64::from)
                });
                Some(rate.filter(|rate| rate.is_finite() && *rate >= 0.0).ok_or_else(|| invalid("pricing.usd_per_mtok", "must be a number of US dollars per million input tokens, zero or more"))?)
            }
        };
        Ok(Self {
            document,
            active_profile,
            profiles,
            shared,
            pricing_usd_per_mtok,
            warnings,
        })
    }

    /// The text to write back. Comments, ordering and spacing a person chose are kept.
    pub(crate) fn to_toml(&self) -> String {
        self.document.to_string()
    }

    /// Every profile name. `default` always exists, and comes first.
    pub(crate) fn profile_names(&self) -> Vec<&str> {
        std::iter::once(DEFAULT_PROFILE)
            .chain(
                self.profiles
                    .keys()
                    .map(String::as_str)
                    .filter(|name| *name != DEFAULT_PROFILE),
            )
            .collect()
    }

    pub(crate) fn has_profile(&self, name: &str) -> bool {
        name == DEFAULT_PROFILE || self.profiles.contains_key(name)
    }

    /// Stores a setting in a profile, creating the profile's table when it is the first one. A
    /// setting every profile shares goes to its own table instead, whatever `profile` says.
    pub(crate) fn set(&mut self, profile: &str, key: Key, new: &Value) {
        self.prepare();
        let table = match key.table() {
            Some(shared) => as_table(self.document.entry(shared).or_insert_with(toml_edit::table)),
            None => self.profile_table(profile),
        };
        let Some(table) = table else {
            return;
        };
        let mut replacement: toml_edit::Value = match new {
            Value::Count(count) => i64::from(*count).into(),
            Value::Switch(on) => (*on).into(),
            text => text.to_string().into(),
        };
        // A comment written beside the old value belongs to the setting, not to the old value.
        if let Some(old) = table.get(key.field()).and_then(Item::as_value) {
            *replacement.decor_mut() = old.decor().clone();
        }
        table[key.field()] = Item::Value(replacement);
        if key.table().is_some() {
            self.shared.insert(key, new.clone());
        } else {
            self.profiles
                .entry(profile.to_owned())
                .or_default()
                .insert(key, new.clone());
        }
    }

    /// Removes a setting from a profile, or from its own table for a setting every profile
    /// shares. Returns whether it was there.
    pub(crate) fn unset(&mut self, profile: &str, key: Key) -> bool {
        if let Some(shared) = key.table() {
            let removed = self.shared.shift_remove(&key).is_some();
            if let Some(table) = self
                .document
                .get_mut(shared)
                .and_then(Item::as_table_like_mut)
            {
                table.remove(key.field());
            }
            return removed;
        }
        let removed = self
            .profiles
            .get_mut(profile)
            .and_then(|values| values.shift_remove(&key))
            .is_some();
        let table = self
            .document
            .get_mut("profiles")
            .and_then(|item| item.get_mut(profile))
            .and_then(Item::as_table_like_mut);
        if let (true, Some(table)) = (removed, table) {
            table.remove(key.name());
        }
        removed
    }

    /// Adds an empty profile. Returns `false` when it already exists.
    pub(crate) fn create_profile(&mut self, name: &str) -> bool {
        if self.profiles.contains_key(name) {
            return false;
        }
        self.prepare();
        self.profile_table(name);
        self.profiles.insert(name.to_owned(), IndexMap::new());
        true
    }

    /// Removes a profile and its settings. If it was the active one, `default` becomes active.
    /// Returns whether there was anything to remove.
    pub(crate) fn delete_profile(&mut self, name: &str) -> bool {
        let removed = self.profiles.shift_remove(name).is_some();
        if let Some(profiles) = self
            .document
            .get_mut("profiles")
            .and_then(Item::as_table_like_mut)
        {
            profiles.remove(name);
        }
        if self.active_profile.as_deref() == Some(name) {
            self.active_profile = None;
            self.document.remove("active_profile");
        }
        removed
    }

    /// Makes a profile the one used when nothing else selects one.
    pub(crate) fn set_active_profile(&mut self, name: &str) {
        self.prepare();
        if name == DEFAULT_PROFILE {
            self.document.remove("active_profile");
            self.active_profile = None;
        } else {
            self.document.insert("active_profile", value(name));
            self.active_profile = Some(name.to_owned());
        }
    }

    /// Gives a brand-new file its header and its `schema_version`.
    ///
    /// A file a person already wrote is left as it is: without the key it is version 1 by
    /// definition, and inserting one would push their opening comment off the top.
    fn prepare(&mut self) {
        if !self.document.is_empty() {
            return;
        }
        self.document
            .insert("schema_version", value(SCHEMA_VERSION));
        if let Some(mut key) = self.document.key_mut("schema_version") {
            key.leaf_decor_mut().set_prefix(HEADER);
        }
    }

    /// The table of one profile, created if need be. `None` is never returned in practice: see
    /// [`as_table`].
    fn profile_table(&mut self, name: &str) -> Option<&mut Table> {
        let profiles = self.document.entry("profiles").or_insert_with(|| {
            let mut table = Table::new();
            // Written as `[profiles.<name>]` headers, without an empty `[profiles]` above them.
            table.set_implicit(true);
            Item::Table(table)
        });
        as_table(
            as_table(profiles)?
                .entry(name)
                .or_insert_with(|| Item::Table(Table::new())),
        )
    }
}

/// The item as a table that can grow. An inline table (`profiles = { ... }`) is rewritten as a
/// standard one, keeping its content; anything else is replaced by an empty table.
///
/// The result is an `Option` only because that is what `toml_edit` returns; the item has just been
/// made a table, so there is always one. Returning `None` rather than panicking keeps a promise:
/// nothing a person puts in their configuration can crash `jev`.
fn as_table(item: &mut Item) -> Option<&mut Table> {
    if !item.is_table() {
        let existing = std::mem::take(item);
        *item = Item::Table(existing.into_table().unwrap_or_default());
    }
    item.as_table_mut()
}

/// Reads the settings every profile shares from their top-level tables, such as `[update]`.
fn read_shared(
    document: &DocumentMut,
    shown_path: &str,
    invalid: &dyn Fn(&str, &str) -> CliError,
    warnings: &mut Vec<Notice>,
) -> Result<IndexMap<Key, Value>, CliError> {
    let mut shared = IndexMap::new();
    for table in SHARED_TABLES {
        let Some(item) = document.get(table) else {
            continue;
        };
        let entries = item
            .as_table_like()
            .ok_or_else(|| invalid(table, "must be a table of settings"))?;
        let known: Vec<Key> = Key::ALL
            .into_iter()
            .filter(|key| key.table() == Some(table))
            .collect();
        for (field, item) in entries.iter() {
            let key_path = format!("{table}.{field}");
            match known.iter().find(|key| key.field() == field) {
                Some(key) => {
                    let parsed =
                        from_toml(*key, item).map_err(|problem| invalid(&key_path, &problem))?;
                    shared.insert(*key, parsed);
                }
                None => warnings.push(unknown_key(
                    shown_path,
                    &key_path,
                    field,
                    &known.iter().map(|key| key.field()).collect::<Vec<_>>(),
                )),
            }
        }
    }
    Ok(shared)
}

/// Reads a setting from the TOML value a person wrote.
fn from_toml(key: Key, item: &Item) -> Result<Value, String> {
    let switch = matches!(key, Key::WarnUnpinned | Key::UpdateAuto);
    let text = match (key, item.as_value()) {
        (Key::Timeout, Some(toml_edit::Value::Integer(seconds))) => seconds.value().to_string(),
        (Key::Timeout, Some(toml_edit::Value::Float(seconds))) => seconds.value().to_string(),
        (Key::MaxRetries | Key::Concurrency, Some(toml_edit::Value::Integer(count))) => {
            count.value().to_string()
        }
        (_, Some(toml_edit::Value::Boolean(on))) if switch => on.value().to_string(),
        (Key::MaxRetries | Key::Concurrency, _) => return Err("must be a whole number".to_owned()),
        _ if switch => return Err("must be true or false".to_owned()),
        (_, Some(toml_edit::Value::String(text))) => text.value().clone(),
        (Key::Timeout, _) => {
            return Err(format!(
                "must be a duration such as \"{}\"",
                duration::format(std::time::Duration::from_secs(30))
            ));
        }
        _ => return Err("must be a string".to_owned()),
    };
    key.parse(&text)
        .map_err(|error| format!("is not valid: {}", error.message))
}

/// The settings a profile holds.
fn profile_keys() -> impl Iterator<Item = Key> {
    Key::ALL.into_iter().filter(|key| key.table().is_none())
}

/// The top-level tables that hold settings every profile shares.
const SHARED_TABLES: [&str; 1] = ["update"];

fn unknown_key(shown_path: &str, key_path: &str, key_name: &str, known: &[&str]) -> Notice {
    let notice = Notice::warning(
        "config_unknown_key",
        format!("{shown_path}: unknown key `{key_path}` is ignored"),
    );
    match suggest::closest(key_name, known.iter().copied()) {
        Some(close) => notice.hint(format!("did you mean `{close}`?")),
        None => notice.hint(format!("the keys allowed there are: {}", known.join(", "))),
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("syntax error")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::ConfigFile;
    use crate::config::settings::{Key, Value};
    use crate::output::Format;

    const PATH: &str = "config.toml";

    #[test]
    fn reads_profiles_and_the_active_profile() {
        let file = ConfigFile::parse(
            r#"
schema_version = 1
active_profile = "staging"

[profiles.default]
model = "jev-1.13.0"
timeout = 45            # seconds
warn_unpinned = false

[profiles.staging]
base_url = "http://127.0.0.1:4010"
output = "yaml"
timeout = "500ms"
max_retries = 0
concurrency = 8
"#,
            PATH,
        )
        .unwrap();

        assert_eq!(file.active_profile.as_deref(), Some("staging"));
        assert_eq!(file.profile_names(), ["default", "staging"]);
        assert_eq!(
            file.profiles["default"][&Key::Timeout],
            Value::Duration(Duration::from_secs(45))
        );
        assert_eq!(
            file.profiles["default"][&Key::WarnUnpinned],
            Value::Switch(false)
        );
        assert_eq!(
            file.profiles["staging"][&Key::Output],
            Value::Output(Format::Yaml)
        );
        assert_eq!(
            file.profiles["staging"][&Key::Timeout],
            Value::Duration(Duration::from_millis(500))
        );
        assert_eq!(file.profiles["staging"][&Key::MaxRetries], Value::Count(0));
        assert!(file.warnings.is_empty());
    }

    #[test]
    fn an_empty_or_missing_file_is_just_the_default_profile() {
        let file = ConfigFile::parse("", PATH).unwrap();

        assert_eq!(file.profile_names(), ["default"]);
        assert!(file.has_profile("default") && !file.has_profile("other"));
        assert_eq!(file.active_profile, None);
    }

    #[test]
    fn unknown_keys_are_warnings_with_a_suggestion_and_reserved_tables_are_silent() {
        let file = ConfigFile::parse(
            "colour = true\n[update]\nauto = false\n[pricing]\nusd_per_mtok = 0.03\n[profiles.default]\nmodle = \"x\"\nmodel = \"jev-1.13.0\"\n",
            PATH,
        )
        .unwrap();

        let warnings: Vec<(&str, Option<&str>)> = file
            .warnings
            .iter()
            .map(|w| (w.message.as_str(), w.hint.as_deref()))
            .collect();
        assert_eq!(
            warnings,
            [
                (
                    "config.toml: unknown key `colour` is ignored",
                    Some(
                        "the keys allowed there are: schema_version, active_profile, profiles, update, pricing"
                    )
                ),
                (
                    "config.toml: unknown key `profiles.default.modle` is ignored",
                    Some("did you mean `model`?")
                ),
            ]
        );
        assert_eq!(
            file.profiles["default"][&Key::Model],
            Value::Text("jev-1.13.0".into())
        );
    }

    #[test]
    fn a_newer_schema_version_is_a_warning_not_a_failure() {
        let file = ConfigFile::parse("schema_version = 7\n", PATH).unwrap();

        assert_eq!(file.warnings.len(), 1);
        assert_eq!(file.warnings[0].code, "config_newer_schema");
    }

    #[test]
    fn a_wrong_value_is_an_error_that_names_the_key() {
        let cases = [
            (
                "[profiles.default]\ntimeout = \"soon\"\n",
                "`profiles.default.timeout`",
            ),
            (
                "[profiles.default]\nmax_retries = \"two\"\n",
                "`profiles.default.max_retries` must be a whole number",
            ),
            (
                "[profiles.default]\nwarn_unpinned = \"yes\"\n",
                "`profiles.default.warn_unpinned` must be true or false",
            ),
            (
                "[profiles.default]\nbase_url = \"ftp://example.com\"\n",
                "`profiles.default.base_url`",
            ),
            (
                "[profiles.default]\nmodel = 3\n",
                "`profiles.default.model` must be a string",
            ),
            ("profiles = 3\n", "`profiles` must be a table of profiles"),
            (
                "active_profile = 3\n",
                "`active_profile` must be a profile name",
            ),
            (
                "schema_version = \"one\"\n",
                "`schema_version` must be a whole number",
            ),
        ];

        for (text, expected) in cases {
            let error = ConfigFile::parse(text, PATH).unwrap_err();
            assert_eq!(error.exit.code(), 2);
            assert!(
                error.message.contains(expected),
                "{text:?}: {}",
                error.message
            );
        }
        let syntax = ConfigFile::parse("[profiles.default\n", PATH).unwrap_err();
        assert!(
            syntax.message.starts_with("config.toml is not valid TOML"),
            "{}",
            syntax.message
        );
    }

    #[test]
    fn a_new_file_gets_a_header_a_schema_version_and_tidy_profile_tables() {
        let mut file = ConfigFile::default();

        file.set("default", Key::Model, &Value::Text("jev-1.13.0".into()));
        file.set("default", Key::MaxRetries, &Value::Count(3));
        file.set(
            "staging",
            Key::Timeout,
            &Value::Duration(Duration::from_millis(500)),
        );
        file.set_active_profile("staging");

        assert_eq!(
            file.to_toml(),
            "# jev configuration. Change it with `jev config set` and `jev profile`, or by hand.\n\
# API keys are never stored in this file.\n\
schema_version = 1\n\
active_profile = \"staging\"\n\
\n\
[profiles.default]\n\
model = \"jev-1.13.0\"\n\
max_retries = 3\n\
\n\
[profiles.staging]\n\
timeout = \"500ms\"\n"
        );
        let reread = ConfigFile::parse(&file.to_toml(), PATH).unwrap();
        assert_eq!(reread.profiles, file.profiles);
        assert_eq!(reread.active_profile.as_deref(), Some("staging"));
    }

    #[test]
    fn editing_keeps_the_comments_and_layout_a_person_wrote() {
        let original = "# my settings\nschema_version = 1\n\n[profiles.default]\n# pinned on purpose\nmodel = \"jev-1.13.0\"   # do not move\ntimeout = 30\n";
        let mut file = ConfigFile::parse(original, PATH).unwrap();

        file.set("default", Key::Model, &Value::Text("jev-1.14.0".into()));
        assert!(file.unset("default", Key::Timeout));
        assert!(!file.unset("default", Key::Timeout), "already gone");

        assert_eq!(
            file.to_toml(),
            "# my settings\nschema_version = 1\n\n[profiles.default]\n# pinned on purpose\nmodel = \"jev-1.14.0\"   # do not move\n"
        );
    }

    #[test]
    fn the_update_settings_live_in_their_own_table_shared_by_every_profile() {
        let file = ConfigFile::parse(
            "[update]\nauto = false\nchannel = \"prerelease\"\npin_version = \"v0.3.1\"\nautomatic = 1\n\
[profiles.default]\n\"update.auto\" = true\n",
            PATH,
        )
        .unwrap();

        assert_eq!(file.shared[&Key::UpdateAuto], Value::Switch(false));
        assert_eq!(
            file.shared[&Key::UpdateChannel],
            Value::Text("prerelease".into())
        );
        assert_eq!(
            file.shared[&Key::UpdatePinVersion],
            Value::Text("0.3.1".into())
        );
        let warnings: Vec<&str> = file.warnings.iter().map(|w| w.message.as_str()).collect();
        assert_eq!(
            warnings,
            [
                "config.toml: unknown key `profiles.default.update.auto` is ignored",
                "config.toml: unknown key `update.automatic` is ignored",
            ]
        );
        assert!(
            file.profiles["default"].is_empty(),
            "a profile cannot hold it"
        );

        for (text, expected) in [
            (
                "[update]\nauto = \"no\"\n",
                "`update.auto` must be true or false",
            ),
            (
                "[update]\nchannel = \"nightly\"\n",
                "`update.channel` is not valid",
            ),
            (
                "[update]\npin_version = 3\n",
                "`update.pin_version` must be a string",
            ),
            ("update = 3\n", "`update` must be a table of settings"),
        ] {
            let error = ConfigFile::parse(text, PATH).unwrap_err();
            assert!(
                error.message.contains(expected),
                "{text:?}: {}",
                error.message
            );
        }
    }

    #[test]
    fn setting_an_update_setting_writes_the_update_table_whatever_the_profile() {
        let original = "# mine\n[profiles.work]\nmodel = \"jev-1.13.0\"\n";
        let mut file = ConfigFile::parse(original, PATH).unwrap();

        file.set("work", Key::UpdateAuto, &Value::Switch(false));
        file.set("work", Key::UpdatePinVersion, &Value::Text("0.3.1".into()));
        assert_eq!(
            file.to_toml(),
            "# mine\n[profiles.work]\nmodel = \"jev-1.13.0\"\n\n[update]\nauto = false\npin_version = \"0.3.1\"\n"
        );
        assert_eq!(file.profiles["work"].len(), 1);

        assert!(file.unset("work", Key::UpdatePinVersion));
        assert!(
            !file.unset("default", Key::UpdatePinVersion),
            "already gone"
        );
        let reread = ConfigFile::parse(&file.to_toml(), PATH).unwrap();
        assert_eq!(reread.shared, file.shared);
        assert_eq!(
            reread.shared.get(&Key::UpdateAuto),
            Some(&Value::Switch(false))
        );
        assert!(!reread.shared.contains_key(&Key::UpdatePinVersion));
    }

    #[test]
    fn profiles_can_be_created_deleted_and_made_active() {
        let mut file = ConfigFile::default();

        assert!(file.create_profile("work"));
        assert!(!file.create_profile("work"), "it already exists");
        file.set_active_profile("work");
        assert_eq!(file.profile_names(), ["default", "work"]);

        assert!(file.delete_profile("work"));
        assert!(!file.delete_profile("work"));
        assert_eq!(
            file.active_profile, None,
            "deleting the active profile falls back to default"
        );
        assert!(!file.to_toml().contains("work"), "{}", file.to_toml());

        file.set_active_profile("default");
        assert!(
            !file.to_toml().contains("active_profile"),
            "default is implicit"
        );
    }
}
