//! `jev completion`: a shell completion script, generated from the tree `jev` parses with, so that
//! it completes exactly the commands and flags `--help` lists.

use clap::ValueEnum;
use clap_complete::Generator;

use super::Context;
use crate::cli::CompletionArgs;
use crate::error::CliError;

pub(crate) fn run(arguments: &CompletionArgs, context: &mut Context<'_>) -> Result<(), CliError> {
    let script = String::from_utf8(script(arguments.shell)).map_err(|error| {
        CliError::internal(format!("the completion script is not UTF-8: {error}"))
    })?;
    // The script is the data: it is printed as is, whatever --output says.
    context.write_raw(&script)
}

/// A shell `jev completion` writes a script for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    /// For Bash, to source from ~/.bashrc
    Bash,
    /// For Zsh, to source from ~/.zshrc or save as `_jev` in a directory on $fpath
    Zsh,
    /// For fish, to save as ~/.config/fish/completions/jev.fish
    Fish,
    /// For PowerShell 5.1 or 7, to load from $PROFILE
    #[value(name = "powershell")]
    PowerShell,
}

impl Shell {
    fn generator(self) -> clap_complete::Shell {
        match self {
            Self::Bash => clap_complete::Shell::Bash,
            Self::Zsh => clap_complete::Shell::Zsh,
            Self::Fish => clap_complete::Shell::Fish,
            Self::PowerShell => clap_complete::Shell::PowerShell,
        }
    }

    /// The name the shell looks the script up by, such as `_jev` for zsh.
    #[must_use]
    pub fn file_name(self) -> String {
        self.generator().file_name(BIN_NAME)
    }
}

const BIN_NAME: &str = "jev";

/// The completion script for `shell`.
pub(crate) fn script(shell: Shell) -> Vec<u8> {
    let mut script = Vec::new();
    clap_complete::generate(
        shell.generator(),
        &mut crate::cli::command(),
        BIN_NAME,
        &mut script,
    );
    script
}

#[cfg(test)]
mod tests {
    use clap::ValueEnum;

    use super::{Shell, script};

    #[test]
    fn every_shell_gets_a_script_that_knows_the_commands() {
        for shell in Shell::value_variants() {
            let script = String::from_utf8(script(*shell)).unwrap();
            for word in ["noul", "choice", "completion", "state-file", "abstain-band"] {
                assert!(script.contains(word), "{shell:?} lacks `{word}`");
            }
        }
    }

    #[test]
    fn file_names_are_the_ones_each_shell_looks_for() {
        assert_eq!(Shell::Bash.file_name(), "jev.bash");
        assert_eq!(Shell::Zsh.file_name(), "_jev");
        assert_eq!(Shell::Fish.file_name(), "jev.fish");
        assert_eq!(Shell::PowerShell.file_name(), "_jev.ps1");
    }
}
