//! `jev version`.

use serde::Serialize;

use super::Context;
use crate::error::CliError;
use crate::output::{Cell, Render, Table, Ui};
use crate::update::{self, AutoUpdate, InstallMethod, Installation};

/// What `jev version` reports.
#[derive(Debug, Serialize)]
pub(crate) struct VersionInfo {
    /// The version of `jev`.
    version: String,
    /// The commit `jev` was built from, or `unknown` outside a git checkout.
    commit: &'static str,
    /// The build date, `YYYY-MM-DD` in UTC.
    build_date: &'static str,
    /// The target triple `jev` was built for.
    target: &'static str,
    /// The version of the `jev-client` library inside.
    client_version: &'static str,
    /// Who installed this binary: `self_managed`, `homebrew`, `cargo` or `unknown`.
    install_method: InstallMethod,
    /// The releases `jev update` follows.
    update_channel: &'static str,
    /// Whether this binary updates itself, and why not when it does not.
    auto_update: AutoUpdate,
}

impl VersionInfo {
    pub(crate) fn current(env: &crate::env::Env) -> Self {
        // Where the binary is cannot fail in practice; if it did, nothing is known about it.
        let install_method = Installation::current()
            .map_or(InstallMethod::Unknown, |installation| {
                InstallMethod::detect(&installation)
            });
        Self {
            version: update::current_version(env).to_string(),
            commit: env!("JEV_BUILD_COMMIT"),
            build_date: env!("JEV_BUILD_DATE"),
            target: env!("JEV_BUILD_TARGET"),
            client_version: jev_client::VERSION,
            install_method,
            update_channel: update::CHANNEL,
            auto_update: AutoUpdate::status(install_method, env),
        }
    }
}

impl Render for VersionInfo {
    fn human(&self, ui: Ui) -> String {
        let auto_update = match (&self.auto_update.enabled, &self.auto_update.reason) {
            (true, _) => "on".to_owned(),
            (false, Some(reason)) => format!("off: {reason}"),
            (false, None) => "off".to_owned(),
        };
        let mut table = Table::default();
        for (label, value) in [
            ("commit", self.commit),
            ("built", self.build_date),
            ("target", self.target),
            ("jev-client", self.client_version),
            ("installed by", self.install_method.name()),
            ("channel", self.update_channel),
            ("auto-update", &auto_update),
        ] {
            table.row(vec![Cell::styled(label, ui.dim(label)), Cell::plain(value)]);
        }
        format!("{} {}\n{}", ui.bold("jev"), self.version, table.render(""))
    }
}

pub(crate) fn run(context: &mut Context<'_>) -> Result<(), CliError> {
    context
        .output
        .emit(&VersionInfo::current(&context.env), context.stdout)
}

#[cfg(test)]
mod tests {
    use super::VersionInfo;
    use crate::env::Env;
    use crate::output::{Render, Ui};

    #[test]
    fn reports_the_version_commit_date_and_target() {
        let info = VersionInfo::current(&Env::default());
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
        let text = VersionInfo::current(&Env::default()).human(Ui::plain());

        assert!(
            text.starts_with(concat!("jev ", env!("CARGO_PKG_VERSION"), "\n")),
            "{text}"
        );
        assert!(text.contains("commit") && text.contains("target"), "{text}");
        assert!(
            text.contains("auto-update") && text.contains("off: "),
            "{text}"
        );
    }

    #[test]
    fn reports_how_it_was_installed_and_whether_it_updates_itself() {
        let json =
            serde_json::to_value(VersionInfo::current(&Env::from([("CI", "true")]))).unwrap();

        // The test binary lives in the build directory, which no installer owns.
        assert_eq!(json["install_method"], "unknown");
        assert_eq!(json["update_channel"], "stable");
        assert_eq!(
            json["auto_update"],
            serde_json::json!({ "enabled": false, "reason": "CI=true" })
        );
    }
}
