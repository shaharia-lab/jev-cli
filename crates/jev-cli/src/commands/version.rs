//! `jev version`.

use serde::Serialize;

use super::Context;
use crate::error::CliError;
use crate::output::{Cell, Render, Table, Ui};

/// What `jev version` reports.
#[derive(Debug, Serialize)]
pub(crate) struct VersionInfo {
    /// The version of `jev`.
    version: &'static str,
    /// The commit `jev` was built from, or `unknown` outside a git checkout.
    commit: &'static str,
    /// The build date, `YYYY-MM-DD` in UTC.
    build_date: &'static str,
    /// The target triple `jev` was built for.
    target: &'static str,
    /// The version of the `jev-client` library inside.
    client_version: &'static str,
}

impl VersionInfo {
    pub(crate) const fn current() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            commit: env!("JEV_BUILD_COMMIT"),
            build_date: env!("JEV_BUILD_DATE"),
            target: env!("JEV_BUILD_TARGET"),
            client_version: jev_client::VERSION,
        }
    }
}

impl Render for VersionInfo {
    fn human(&self, ui: Ui) -> String {
        let mut table = Table::default();
        for (label, value) in [
            ("commit", self.commit),
            ("built", self.build_date),
            ("target", self.target),
            ("jev-client", self.client_version),
        ] {
            table.row(vec![Cell::styled(label, ui.dim(label)), Cell::plain(value)]);
        }
        format!("{} {}\n{}", ui.bold("jev"), self.version, table.render(""))
    }
}

pub(crate) fn run(context: &mut Context<'_>) -> Result<(), CliError> {
    context.output.emit(&VersionInfo::current(), context.stdout)
}

#[cfg(test)]
mod tests {
    use super::VersionInfo;
    use crate::output::{Render, Ui};

    #[test]
    fn reports_the_version_commit_date_and_target() {
        let info = VersionInfo::current();
        let json = serde_json::to_value(&info).unwrap();

        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(json["client_version"], jev_client::VERSION);
        assert!(!json["commit"].as_str().unwrap().is_empty());
        assert!(
            json["target"].as_str().unwrap().contains('-'),
            "a target triple: {}",
            json["target"]
        );

        let date = json["build_date"].as_str().unwrap();
        let parts: Vec<&str> = date.split('-').collect();
        assert!(
            parts.len() == 3 && parts.iter().all(|part| part.parse::<u32>().is_ok()),
            "{date}"
        );
    }

    #[test]
    fn the_human_form_leads_with_name_and_version() {
        let text = VersionInfo::current().human(Ui::plain());

        assert!(
            text.starts_with(concat!("jev ", env!("CARGO_PKG_VERSION"), "\n")),
            "{text}"
        );
        assert!(text.contains("commit") && text.contains("target"), "{text}");
    }
}
