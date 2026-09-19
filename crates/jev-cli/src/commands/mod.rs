//! One module per command group. `run` is the only place that knows which module handles what.

#[cfg(feature = "internal-test-hooks")]
pub(crate) mod debug;
mod version;

use std::io::Write;

use crate::cli::{
    AuthCommand, BatchCommand, Command, ConfigCommand, McpCommand, ModelsCommand, ProfileCommand,
    SchemaCommand,
};
use crate::error::CliError;
use crate::interaction::Interaction;
use crate::output::Output;

/// What a command needs from the outside world.
// `interaction` and `stdin` are first read outside the test hooks by `jev auth login` and
// `jev eval` (issues #10 and #9).
#[cfg_attr(not(feature = "internal-test-hooks"), allow(dead_code))]
pub(crate) struct Context<'a> {
    pub(crate) output: Output,
    pub(crate) interaction: Interaction,
    pub(crate) stdout: &'a mut dyn Write,
    pub(crate) stdin: &'a mut dyn std::io::Read,
}

/// Runs a command.
///
/// # Errors
///
/// Whatever the command fails with. The caller prints it and turns it into an exit code.
pub(crate) fn run(command: &Command, context: &mut Context<'_>) -> Result<(), CliError> {
    match command {
        Command::Version => version::run(context),
        #[cfg(feature = "internal-test-hooks")]
        Command::Debug(command) => debug::run(command, context),
        pending => {
            let (name, issue) = pending_command(pending);
            Err(CliError::not_implemented(name, issue))
        }
    }
}

/// The name of a command that has no behaviour yet, and the issue that will give it some.
const fn pending_command(command: &Command) -> (&'static str, u32) {
    match command {
        Command::Eval(_) => ("eval", 9),
        Command::Noul(_) => ("noul", 12),
        Command::Choice(_) => ("choice", 12),
        Command::Score(_) => ("score", 12),
        Command::Validate(_) => ("validate", 11),
        Command::Batch(BatchCommand::Run(_)) => ("batch run", 13),
        Command::Models(ModelsCommand::List(_)) => ("models list", 9),
        Command::Auth(AuthCommand::Login(_)) => ("auth login", 10),
        Command::Auth(AuthCommand::Status(_)) => ("auth status", 10),
        Command::Auth(AuthCommand::Logout(_)) => ("auth logout", 10),
        Command::Config(ConfigCommand::Get(_)) => ("config get", 8),
        Command::Config(ConfigCommand::Set(_)) => ("config set", 8),
        Command::Config(ConfigCommand::Unset(_)) => ("config unset", 8),
        Command::Config(ConfigCommand::List(_)) => ("config list", 8),
        Command::Config(ConfigCommand::Path(_)) => ("config path", 8),
        Command::Profile(ProfileCommand::List(_)) => ("profile list", 8),
        Command::Profile(ProfileCommand::Use(_)) => ("profile use", 8),
        Command::Profile(ProfileCommand::Create(_)) => ("profile create", 8),
        Command::Profile(ProfileCommand::Delete(_)) => ("profile delete", 8),
        Command::Schema(SchemaCommand::Request(_)) => ("schema request", 16),
        Command::Schema(SchemaCommand::Questions(_)) => ("schema questions", 16),
        Command::Schema(SchemaCommand::BatchRecord(_)) => ("schema batch-record", 16),
        Command::Schema(SchemaCommand::Output(_)) => ("schema output", 16),
        Command::Schema(SchemaCommand::Error(_)) => ("schema error", 16),
        Command::Spec(_) => ("spec", 15),
        Command::Mcp(McpCommand::Serve(_)) => ("mcp serve", 18),
        Command::Update(_) => ("update", 25),
        Command::Completion(_) => ("completion", 17),
        Command::Version => ("version", 7),
        #[cfg(feature = "internal-test-hooks")]
        Command::Debug(_) => ("debug", 7),
    }
}
