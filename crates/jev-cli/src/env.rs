//! A snapshot of the environment, so that everything reading it can be tested without touching
//! the real one.

use std::collections::HashMap;
use std::fmt;

/// Variables `jev` reads that do not start with `JEV_` or `TYPESAFE_`.
const OTHER_VARIABLES: [&str; 10] = [
    "CI",
    "NO_COLOR",
    "TERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "HOME",
    "XDG_CONFIG_HOME",
    "APPDATA",
    "USERPROFILE",
];

/// The environment variables `jev` reads, as they were when it started. An empty value counts as
/// unset, which is how every one of them is documented to behave.
///
/// Only variables `jev` has a use for are kept, and `Debug` shows names but never values: one of
/// them is `TYPESAFE_API_KEY`.
#[derive(Clone, Default)]
pub(crate) struct Env(HashMap<String, String>);

impl Env {
    /// The process environment. Variables that are not valid Unicode are ignored.
    pub(crate) fn from_process() -> Self {
        Self(
            std::env::vars_os()
                .filter_map(|(name, value)| {
                    Some((name.into_string().ok()?, value.into_string().ok()?))
                })
                .filter(|(name, _)| {
                    name.starts_with("JEV_")
                        || name.starts_with("TYPESAFE_")
                        || OTHER_VARIABLES.contains(&name.as_str())
                })
                .collect(),
        )
    }

    /// The value of a variable, unless it is unset or empty.
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.0
            .get(name)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    }
}

impl fmt::Debug for Env {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut names: Vec<&str> = self.0.keys().map(String::as_str).collect();
        names.sort_unstable();
        formatter.debug_tuple("Env").field(&names).finish()
    }
}

#[cfg(test)]
impl<const N: usize> From<[(&str, &str); N]> for Env {
    fn from(variables: [(&str, &str); N]) -> Self {
        Self(
            variables
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Env;

    #[test]
    fn an_empty_variable_counts_as_unset() {
        let env = Env::from([("JEV_OUTPUT", "json"), ("JEV_PROFILE", "")]);

        assert_eq!(env.get("JEV_OUTPUT"), Some("json"));
        assert_eq!(env.get("JEV_PROFILE"), None);
        assert_eq!(env.get("MISSING"), None);
    }

    #[test]
    fn debug_shows_names_and_never_values() {
        let env = Env::from([
            ("TYPESAFE_API_KEY", "sentinel-key-value"),
            ("JEV_OUTPUT", "json"),
        ]);

        let shown = format!("{env:?} {env:#?}");

        assert!(
            shown.contains("TYPESAFE_API_KEY") && shown.contains("JEV_OUTPUT"),
            "{shown}"
        );
        assert!(
            !shown.contains("sentinel-key-value") && !shown.contains("json"),
            "{shown}"
        );
    }
}
