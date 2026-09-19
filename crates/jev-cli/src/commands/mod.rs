//! One module per command group. `run` is the only place that knows which module handles what.

mod config;
#[cfg(feature = "internal-test-hooks")]
pub(crate) mod debug;
mod profile;
mod version;

use std::io::Write;

use crate::cli::{AuthCommand, BatchCommand, Command, McpCommand, ModelsCommand, SchemaCommand};
use crate::config::{ConfigFile, ConfigStore, Flags, Settings};
use crate::error::CliError;
use crate::interaction::Interaction;
use crate::output::Output;

/// The configuration as it was found when `jev` started.
///
/// Each part is kept as a `Result`, not unwrapped at start-up, so that a broken `config.toml` only
/// stops the commands that need it. `jev version` and `jev config path` must keep working, or
/// there would be no way to find the file that needs fixing.
pub(crate) struct Configuration {
    pub(crate) store: Result<ConfigStore, CliError>,
    pub(crate) loaded: Result<(ConfigFile, Settings), CliError>,
    /// The values given on the command line, which `jev profile create` stores.
    pub(crate) flags: Flags,
}

/// What a command needs from the outside world.
// `interaction` and `stdin` are first read outside the test hooks by `jev auth login` and
// `jev eval` (issues #10 and #9).
#[cfg_attr(not(feature = "internal-test-hooks"), allow(dead_code))]
pub(crate) struct Context<'a> {
    pub(crate) output: Output,
    pub(crate) interaction: Interaction,
    pub(crate) configuration: Configuration,
    pub(crate) stdout: &'a mut dyn Write,
    pub(crate) stdin: &'a mut dyn std::io::Read,
}

impl Context<'_> {
    /// The effective settings.
    ///
    /// # Errors
    ///
    /// Whatever stopped the configuration from loading.
    pub(crate) fn settings(&self) -> Result<&Settings, CliError> {
        self.configuration
            .loaded
            .as_ref()
            .map(|(_, settings)| settings)
            .map_err(Clone::clone)
    }

    /// The configuration file as it was read.
    ///
    /// # Errors
    ///
    /// Whatever stopped the configuration from loading.
    pub(crate) fn config_file(&self) -> Result<&ConfigFile, CliError> {
        self.configuration
            .loaded
            .as_ref()
            .map(|(file, _)| file)
            .map_err(Clone::clone)
    }

    /// Where the configuration lives.
    ///
    /// # Errors
    ///
    /// A usage error when the configuration directory cannot be worked out.
    pub(crate) fn store(&self) -> Result<&ConfigStore, CliError> {
        self.configuration.store.as_ref().map_err(Clone::clone)
    }
}

/// Runs a command.
///
/// # Errors
///
/// Whatever the command fails with. The caller prints it and turns it into an exit code.
pub(crate) fn run(command: &Command, context: &mut Context<'_>) -> Result<(), CliError> {
    match command {
        Command::Version => version::run(context),
        Command::Config(command) => config::run(command, context),
        Command::Profile(command) => profile::run(command, context),
        #[cfg(feature = "internal-test-hooks")]
        Command::Debug(command) => debug::run(command, context),
        Command::Eval(_) => pending("eval", 9),
        Command::Noul(_) => pending("noul", 12),
        Command::Choice(_) => pending("choice", 12),
        Command::Score(_) => pending("score", 12),
        Command::Validate(_) => pending("validate", 11),
        Command::Batch(BatchCommand::Run(_)) => pending("batch run", 13),
        Command::Models(ModelsCommand::List(_)) => pending("models list", 9),
        Command::Auth(AuthCommand::Login(_)) => pending("auth login", 10),
        Command::Auth(AuthCommand::Status(_)) => pending("auth status", 10),
        Command::Auth(AuthCommand::Logout(_)) => pending("auth logout", 10),
        Command::Schema(SchemaCommand::Request(_)) => pending("schema request", 16),
        Command::Schema(SchemaCommand::Questions(_)) => pending("schema questions", 16),
        Command::Schema(SchemaCommand::BatchRecord(_)) => pending("schema batch-record", 16),
        Command::Schema(SchemaCommand::Output(_)) => pending("schema output", 16),
        Command::Schema(SchemaCommand::Error(_)) => pending("schema error", 16),
        Command::Spec(_) => pending("spec", 15),
        Command::Mcp(McpCommand::Serve(_)) => pending("mcp serve", 18),
        Command::Update(_) => pending("update", 25),
        Command::Completion(_) => pending("completion", 17),
    }
}

/// A command that is in the tree but has no behaviour yet, and the issue that will give it some.
fn pending(name: &str, issue: u32) -> Result<(), CliError> {
    Err(CliError::not_implemented(name, issue))
}
