//! One module per command group. `run` is the only place that knows which module handles what.

mod config;
#[cfg(feature = "internal-test-hooks")]
pub(crate) mod debug;
mod eval;
mod models;
mod profile;
mod version;

use std::io::Write;

use crate::cli::{AuthCommand, BatchCommand, Command, McpCommand, ModelsCommand, SchemaCommand};
use jev_client::HttpTransport;

use crate::client::{self, Connection};
use crate::config::{ConfigFile, ConfigStore, Flags, Settings};
use crate::credentials::{CredentialStore, EnvCredentials};
use crate::env::Env;
use crate::error::CliError;
use crate::interaction::Interaction;
use crate::notice::{Notice, Notifier};
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
pub(crate) struct Context<'a> {
    pub(crate) output: Output,
    pub(crate) interaction: Interaction,
    pub(crate) configuration: Configuration,
    pub(crate) env: Env,
    pub(crate) connection: Connection,
    pub(crate) notifier: Notifier,
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

    /// Says something on stderr that is neither the result nor a failure.
    pub(crate) fn notify(&self, notice: &Notice) {
        self.notifier.emit(notice);
    }

    /// The transport for the selected profile, with its API key.
    ///
    /// # Errors
    ///
    /// An authentication error when there is no usable key, or a usage error for a bad base URL.
    pub(crate) fn transport(&self, settings: &Settings) -> Result<HttpTransport, CliError> {
        let (api_key, _source) = EnvCredentials(&self.env).api_key(settings.profile_name())?;
        let (transport, notices) = client::transport(settings, self.connection, api_key)?;
        for notice in &notices {
            self.notify(notice);
        }
        Ok(transport)
    }

    /// Writes text to stdout as it is, ending it with a newline if it lacks one.
    ///
    /// # Errors
    ///
    /// An internal error when stdout cannot be written to. A closed pipe is not an error.
    pub(crate) fn write_raw(&mut self, text: &str) -> Result<(), CliError> {
        let newline = if text.ends_with('\n') { "" } else { "\n" };
        match write!(self.stdout, "{text}{newline}").and_then(|()| self.stdout.flush()) {
            Err(error) if error.kind() != std::io::ErrorKind::BrokenPipe => Err(
                CliError::internal(format!("could not write to stdout: {error}")),
            ),
            _ => Ok(()),
        }
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
        Command::Eval(arguments) => eval::run(arguments, context),
        Command::Noul(_) => pending("noul", 12),
        Command::Choice(_) => pending("choice", 12),
        Command::Score(_) => pending("score", 12),
        Command::Validate(_) => pending("validate", 11),
        Command::Batch(BatchCommand::Run(_)) => pending("batch run", 13),
        Command::Models(ModelsCommand::List) => models::list(context),
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
