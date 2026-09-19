//! How this `jev` was installed, which decides who updates it.

use std::path::Path;

use semver::Version;
use serde::Serialize;

use super::Installation;

/// Who installed the running binary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InstallMethod {
    /// The install script, which leaves a receipt in the updater's directory: `jev update`
    /// replaces the binary.
    SelfManaged,
    /// A Homebrew formula: Homebrew updates it.
    Homebrew,
    /// `cargo install`: cargo updates it.
    Cargo,
    /// None of these, such as an archive unpacked by hand. `jev update` replaces the binary when
    /// asked to.
    Unknown,
}

impl InstallMethod {
    /// Works out how the binary of `installation` was installed, from where it is.
    pub(crate) fn detect(installation: &Installation) -> Self {
        let exe = installation.exe();
        // Homebrew installs a formula into <prefix>/Cellar/<formula>/<version>/, and links it from
        // <prefix>/bin; the installation's path has had links resolved.
        if exe.components().any(|part| part.as_os_str() == "Cellar") {
            return Self::Homebrew;
        }
        // `cargo install` puts binaries in <root>/bin and records them in <root>/.crates.toml.
        let root = exe
            .parent()
            .filter(|dir| dir.file_name().is_some_and(|name| name == "bin"))
            .and_then(Path::parent);
        if root.is_some_and(|root| {
            root.join(".crates.toml").is_file() || root.join(".crates2.json").is_file()
        }) {
            return Self::Cargo;
        }
        if installation.receipt().is_file() {
            return Self::SelfManaged;
        }
        Self::Unknown
    }

    /// The stable name, as `jev version` and `jev update` report it.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::SelfManaged => "self_managed",
            Self::Homebrew => "homebrew",
            Self::Cargo => "cargo",
            Self::Unknown => "unknown",
        }
    }

    /// The package manager that owns the install, when one does.
    pub(crate) const fn package_manager(self) -> Option<&'static str> {
        match self {
            Self::Homebrew => Some("Homebrew"),
            Self::Cargo => Some("cargo"),
            Self::SelfManaged | Self::Unknown => None,
        }
    }

    /// The command that updates a package-manager install, to the latest version or to `version`.
    pub(crate) fn upgrade_command(self, version: Option<&Version>) -> Option<String> {
        match (self, version) {
            (Self::Homebrew, None) => Some("brew upgrade shaharia-lab/tap/jev".to_owned()),
            (Self::Homebrew, Some(version)) => {
                Some(format!("brew install shaharia-lab/tap/jev@{version}"))
            }
            (Self::Cargo, None) => Some("cargo install jev-cli --locked".to_owned()),
            (Self::Cargo, Some(version)) => Some(format!(
                "cargo install jev-cli --locked --version {version}"
            )),
            (Self::SelfManaged | Self::Unknown, _) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use semver::Version;

    use super::InstallMethod;
    use crate::update::Installation;
    use crate::update::tests::scratch;

    fn detect(relative: &str, markers: &[&str]) -> InstallMethod {
        let root = scratch();
        for marker in markers {
            let path = root.join(marker);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }
        let exe = root.join(relative);
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, "binary").unwrap();
        InstallMethod::detect(&Installation::new(exe))
    }

    #[test]
    fn the_install_method_follows_from_the_path_and_its_neighbours() {
        let cases = [
            (
                "homebrew/Cellar/jev/0.1.0/bin/jev",
                &[][..],
                InstallMethod::Homebrew,
            ),
            (
                ".cargo/bin/jev",
                &[".cargo/.crates.toml"],
                InstallMethod::Cargo,
            ),
            (
                "tools/bin/jev",
                &["tools/.crates2.json"],
                InstallMethod::Cargo,
            ),
            (
                ".local/bin/jev",
                &[".local/bin/.jev-update/receipt.json"],
                InstallMethod::SelfManaged,
            ),
            // A bin directory is not enough for cargo, nor a receipt somewhere else.
            (".cargo/bin/jev", &[], InstallMethod::Unknown),
            ("elsewhere/jev", &["receipt.json"], InstallMethod::Unknown),
        ];

        for (path, markers, expected) in cases {
            assert_eq!(detect(path, markers), expected, "{path}");
        }
    }

    #[test]
    fn package_manager_installs_name_their_upgrade_command() {
        let version = Version::new(0, 3, 1);

        assert_eq!(
            InstallMethod::Homebrew.upgrade_command(None).as_deref(),
            Some("brew upgrade shaharia-lab/tap/jev")
        );
        assert_eq!(
            InstallMethod::Homebrew
                .upgrade_command(Some(&version))
                .as_deref(),
            Some("brew install shaharia-lab/tap/jev@0.3.1")
        );
        assert_eq!(
            InstallMethod::Cargo
                .upgrade_command(Some(&version))
                .as_deref(),
            Some("cargo install jev-cli --locked --version 0.3.1")
        );
        assert_eq!(InstallMethod::SelfManaged.upgrade_command(None), None);
        assert_eq!(InstallMethod::Unknown.package_manager(), None);
    }
}
