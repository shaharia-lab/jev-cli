//! A snapshot of the environment, so that everything reading it can be tested without touching
//! the real one.

use std::collections::HashMap;

/// Environment variables as they were when `jev` started. An empty value counts as unset, which
/// is how every variable `jev` reads is documented to behave.
#[derive(Clone, Debug, Default)]
pub(crate) struct Env(HashMap<String, String>);

impl Env {
    /// The process environment. Variables that are not valid Unicode are ignored.
    pub(crate) fn from_process() -> Self {
        Self(
            std::env::vars_os()
                .filter_map(|(name, value)| {
                    Some((name.into_string().ok()?, value.into_string().ok()?))
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
}
