//! One module per command group. `run` is the only place that knows which module handles what.

mod auth;
mod batch;
mod config;
#[cfg(feature = "internal-test-hooks")]
pub(crate) mod debug;
mod eval;
mod mcp;
mod models;
mod profile;
mod schema;
mod shortcut;
mod spec;
mod validate;
mod version;

use std::io::Write;

use crate::cli::{BatchCommand, Command, McpCommand, ModelsCommand};
use jev_client::HttpTransport;

use crate::client::{self, Connection};
use crate::config::{ConfigFile, ConfigStore, Flags, Settings};
use crate::credentials::{CredentialStore, Credentials};
use crate::env::Env;
use crate::error::CliError;
use crate::exit::Exit;
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
        let credentials = Credentials::new(&self.env, self.store()?.dir());
        let (api_key, _source) = credentials.api_key(settings.profile_name())?;
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
pub(crate) fn run(command: &Command, context: &mut Context<'_>) -> Result<Exit, CliError> {
    match command {
        // The evaluating commands decide their own exit code: a gate may make it 10 or 11.
        Command::Eval(arguments) => return eval::run(arguments, context),
        Command::Noul(arguments) => return shortcut::noul(arguments, context),
        Command::Choice(arguments) => return shortcut::choice(arguments, context),
        Command::Score(arguments) => return shortcut::score(arguments, context),
        // A batch with failed rows exits 7.
        Command::Batch(BatchCommand::Run(arguments)) => return batch::run(arguments, context),
        Command::Version => version::run(context),
        Command::Spec => spec::run(context),
        Command::Auth(command) => auth::run(command, context),
        Command::Config(command) => config::run(command, context),
        Command::Profile(command) => profile::run(command, context),
        Command::Validate(arguments) => validate::run(arguments, context),
        Command::Models(ModelsCommand::List) => models::list(context),
        #[cfg(feature = "internal-test-hooks")]
        Command::Debug(command) => debug::run(command, context),
        Command::Schema(command) => schema::run(command, context),
        Command::Mcp(McpCommand::Serve(arguments)) => mcp::serve(arguments, context),
        Command::Update(_) => pending("update", 25),
        Command::Completion(_) => pending("completion", 17),
    }
    .map(|()| Exit::Success)
}

/// A command that is in the tree but has no behaviour yet, and the issue that will give it some.
fn pending(name: &str, issue: u32) -> Result<(), CliError> {
    Err(CliError::not_implemented(name, issue))
}
