//! Where the configuration lives.

use std::path::PathBuf;

use crate::env::Env;
use crate::error::CliError;

/// The name of the configuration file inside the configuration directory.
pub(crate) const CONFIG_FILE: &str = "config.toml";

/// The configuration directory: `JEV_CONFIG_DIR` when set, otherwise the platform's.
///
/// - Linux and other Unix: `$XDG_CONFIG_HOME/jev`, or `~/.config/jev`.
/// - macOS: `~/Library/Application Support/jev`.
/// - Windows: `%APPDATA%\jev`.
///
/// # Errors
///
/// A usage error when the platform variables that locate it are missing, naming the override.
pub(crate) fn config_dir(env: &Env) -> Result<PathBuf, CliError> {
    if let Some(dir) = env.get("JEV_CONFIG_DIR") {
        return Ok(PathBuf::from(dir));
    }
    platform_dir(env, Platform::current()).ok_or_else(|| {
        CliError::usage("cannot work out where the configuration directory is")
            .hint("set JEV_CONFIG_DIR to a directory jev may use for its configuration")
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    Windows,
    MacOs,
    Unix,
}

impl Platform {
    const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Unix
        }
    }
}

fn platform_dir(env: &Env, platform: Platform) -> Option<PathBuf> {
    let home = || env.get("HOME").map(PathBuf::from);
    let base = match platform {
        Platform::Windows => PathBuf::from(env.get("APPDATA")?),
        Platform::MacOs => home()?.join("Library").join("Application Support"),
        // The XDG specification says a relative path here is invalid and must be ignored.
        Platform::Unix => match env
            .get("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
        {
            Some(xdg) => xdg,
            None => home()?.join(".config"),
        },
    };
    Some(base.join("jev"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Platform, config_dir, platform_dir};
    use crate::env::Env;

    #[test]
    fn the_override_wins_on_every_platform() {
        let env = Env::from([
            ("JEV_CONFIG_DIR", "/tmp/jev-test"),
            ("HOME", "/home/u"),
            ("APPDATA", "C:\\Users\\u"),
        ]);

        assert_eq!(config_dir(&env).unwrap(), PathBuf::from("/tmp/jev-test"));
    }

    #[test]
    fn each_platform_uses_its_own_convention() {
        let env = Env::from([
            ("HOME", "/home/u"),
            ("APPDATA", "C:\\Users\\u\\AppData\\Roaming"),
        ]);

        assert_eq!(
            platform_dir(&env, Platform::Unix),
            Some(PathBuf::from("/home/u/.config/jev"))
        );
        assert_eq!(
            platform_dir(&env, Platform::MacOs),
            Some(PathBuf::from("/home/u/Library/Application Support/jev"))
        );
        assert_eq!(
            platform_dir(&env, Platform::Windows),
            Some(PathBuf::from("C:\\Users\\u\\AppData\\Roaming").join("jev"))
        );
    }

    #[test]
    fn xdg_config_home_is_honoured_only_when_absolute() {
        // What counts as absolute depends on the platform running the test: `/etc` is not
        // absolute on Windows, where a path needs a drive.
        let xdg = if cfg!(windows) {
            "C:\\xdg-u"
        } else {
            "/etc/xdg-u"
        };
        let absolute = Env::from([("HOME", "/home/u"), ("XDG_CONFIG_HOME", xdg)]);
        let relative = Env::from([("HOME", "/home/u"), ("XDG_CONFIG_HOME", "relative/dir")]);

        assert_eq!(
            platform_dir(&absolute, Platform::Unix),
            Some(PathBuf::from(xdg).join("jev"))
        );
        assert_eq!(
            platform_dir(&relative, Platform::Unix),
            Some(PathBuf::from("/home/u").join(".config").join("jev"))
        );
    }

    #[test]
    fn a_missing_home_is_an_error_that_names_the_override() {
        let error = config_dir(&Env::default()).unwrap_err();

        assert_eq!(error.exit.code(), 2);
        assert!(error.hint.unwrap().contains("JEV_CONFIG_DIR"));
    }
}
