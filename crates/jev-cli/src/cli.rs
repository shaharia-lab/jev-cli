//! The command tree. `clap` is the single source for parsing, `--help` and, later, `jev spec`.

use std::ffi::OsString;
use std::time::Duration;

use clap::builder::FalseyValueParser;
use clap::{ArgAction, Args, Parser, Subcommand};

use crate::output::Format;

/// Unofficial command-line tool for TypeSafe AI's Jev model.
#[derive(Debug, Parser)]
#[command(
    name = "jev",
    // Without this, help and errors name the program after argv[0], which is `jev.exe` on Windows.
    // Help, and the command spec generated from it, should read the same everywhere.
    bin_name = "jev",
    version,
    about = "Ask TypeSafe AI's Jev model typed questions from a terminal, a script or an AI agent (unofficial)",
    long_about = "jev sends a `state` (text or structured data) and typed questions to TypeSafe AI's Jev \
model and returns calibrated probabilities, never generated text.\n\n\
Three question types: `noul` (yes/no, the probability of yes), `choice` (one of up to 255 options) and \
`score` (a position on a rubric of 2 to 10 levels).\n\n\
stdout carries data only: readable text on a terminal, JSON when piped. Everything else goes to stderr. \
Exit codes are a stable contract: 0 ok, 2 usage, 3 auth, 4 API rejected, 5 rate limited, 6 network, \
10 gate condition false.\n\n\
This is an unofficial community tool, not affiliated with TypeSafe AI.",
    arg_required_else_help = true,
    subcommand_required = true,
    disable_help_subcommand = true,
    max_term_width = 100
)]
pub(crate) struct Cli {
    #[command(flatten)]
    pub(crate) global: GlobalArgs,

    #[command(subcommand)]
    pub(crate) command: Command,
}

/// Flags accepted by every command.
// On/off command-line switches are booleans by nature; there is no state machine hiding here.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default, Args)]
#[command(next_help_heading = "Global options")]
pub(crate) struct GlobalArgs {
    /// Named profile to use (its API key, base URL and defaults) [env: `JEV_PROFILE`]
    #[arg(long, global = true, value_name = "NAME")]
    pub(crate) profile: Option<String>,

    /// Output format [default: table on a terminal, json when piped] [env: `JEV_OUTPUT`]
    #[arg(
        short = 'o',
        long,
        global = true,
        value_enum,
        ignore_case = true,
        value_name = "FORMAT"
    )]
    pub(crate) output: Option<Format>,

    /// Print one raw value from the output, e.g. `answers.is_urgent.noul`
    #[arg(long, global = true, value_name = "PATH")]
    pub(crate) field: Option<String>,

    /// Model alias or versioned id, e.g. `jev-latest` or `jev-1.13.0`
    #[arg(long, global = true, value_name = "MODEL")]
    pub(crate) model: Option<String>,

    // Not a doc comment: rustdoc wants the URL in angle brackets, which would show up in --help.
    #[arg(
        long,
        global = true,
        value_name = "URL",
        help = "API root [default: https://api.typesafe.ai]"
    )]
    pub(crate) base_url: Option<String>,

    /// Time allowed per attempt, e.g. `30`, `30s`, `500ms` or `2m` [default: 30s]
    #[arg(long, global = true, value_name = "DURATION", value_parser = crate::duration::parse)]
    pub(crate) timeout: Option<Duration>,

    /// Retries after the first attempt; 0 disables retrying [default: 2]
    #[arg(long, global = true, value_name = "N")]
    pub(crate) max_retries: Option<u32>,

    /// Never ask for input; fail with an error naming the flag to use instead
    #[arg(long, global = true, env = "JEV_NO_INPUT", value_parser = FalseyValueParser::new())]
    pub(crate) no_input: bool,

    /// Disable colour (also honours the `NO_COLOR` environment variable)
    #[arg(long, global = true)]
    pub(crate) no_color: bool,

    /// Use only ASCII characters in human-readable output
    #[arg(long, global = true)]
    pub(crate) ascii: bool,

    /// Print errors only; suppress notices and warnings on stderr
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    pub(crate) quiet: bool,

    /// Log to stderr; repeat for more (-v requests and retries, -vv everything)
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub(crate) verbose: u8,
}

/// Arguments of a command whose behaviour has not been written yet. Everything is accepted so
/// that the answer is always "not implemented", never a complaint about a flag.
#[derive(Debug, Args)]
pub(crate) struct Pending {
    #[arg(hide = true, num_args = 0.., trailing_var_arg = true, allow_hyphen_values = true)]
    pub(crate) rest: Vec<OsString>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Evaluate a full request (state + many questions) in one API call
    Eval(Pending),
    /// Ask one yes/no question; returns the probability of yes
    Noul(Pending),
    /// Pick one option from a set you define
    Choice(Pending),
    /// Rate the state on an ordered rubric of 2 to 10 levels
    Score(Pending),
    /// Check a request file offline; nothing is sent or billed
    Validate(Pending),
    /// Apply one question set to every row of a JSONL or CSV file
    #[command(subcommand)]
    Batch(BatchCommand),
    /// List the models available to the account
    #[command(subcommand)]
    Models(ModelsCommand),
    /// Store, inspect or remove the API key for a profile
    #[command(subcommand)]
    Auth(AuthCommand),
    /// Read and change configuration
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Manage named profiles
    #[command(subcommand)]
    Profile(ProfileCommand),
    /// Print JSON Schemas for request files and for jev's own output
    #[command(subcommand)]
    Schema(SchemaCommand),
    /// Dump the entire command tree as JSON, for agents and tooling
    Spec(Pending),
    /// Run as a Model Context Protocol server
    #[command(subcommand)]
    Mcp(McpCommand),
    /// Update jev, check for an update, or roll one back
    Update(Pending),
    /// Print a shell completion script
    Completion(Pending),
    /// Print the version, commit, build date and target
    Version,
    /// Internal hooks for jev's own test suite. Not part of any release build.
    #[cfg(feature = "internal-test-hooks")]
    #[command(subcommand, hide = true)]
    Debug(crate::commands::debug::DebugCommand),
}

#[derive(Debug, Subcommand)]
pub(crate) enum BatchCommand {
    /// Evaluate every row of an input file, writing one result record per row
    Run(Pending),
}

#[derive(Debug, Subcommand)]
pub(crate) enum ModelsCommand {
    /// List model names and aliases, with their description and release date
    List(Pending),
}

#[derive(Debug, Subcommand)]
pub(crate) enum AuthCommand {
    /// Store an API key for the profile
    Login(Pending),
    /// Show whether a key is configured, and where it comes from
    Status(Pending),
    /// Remove the stored API key
    Logout(Pending),
}

#[derive(Debug, Subcommand)]
pub(crate) enum ConfigCommand {
    /// Print the effective value of one setting, and where it comes from
    Get {
        /// Setting name: `base_url`, `model`, `output`, `timeout`, `max_retries`, `concurrency` or `warn_unpinned`
        key: String,
    },
    /// Store a setting in the selected profile
    Set {
        /// Setting name: `base_url`, `model`, `output`, `timeout`, `max_retries`, `concurrency` or `warn_unpinned`
        key: String,
        /// The new value, e.g. `jev-1.13.0`, `45s`, `json` or `false`
        value: String,
    },
    /// Remove a setting from the selected profile, restoring its default
    Unset {
        /// Setting name: `base_url`, `model`, `output`, `timeout`, `max_retries`, `concurrency` or `warn_unpinned`
        key: String,
    },
    /// Print every effective setting and where its value comes from (flag, env, profile or default)
    List,
    /// Print the path of the configuration file
    Path,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ProfileCommand {
    /// List the profiles, marking the active one
    List,
    /// Make a profile the one used when neither --profile nor `JEV_PROFILE` selects another
    Use {
        /// Name of an existing profile
        name: String,
    },
    /// Create a profile; --base-url, --model, --output, --timeout and --max-retries become its settings
    Create {
        /// Name for the new profile: letters, digits, `-` and `_`
        name: String,
    },
    /// Delete a profile and its settings
    Delete {
        /// Name of the profile to delete
        name: String,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum SchemaCommand {
    /// Schema of a request file
    Request(Pending),
    /// Schema of a questions file
    Questions(Pending),
    /// Schema of one batch output record
    BatchRecord(Pending),
    /// Schema of the result envelope
    Output(Pending),
    /// Schema of a JSON error
    Error(Pending),
}

#[derive(Debug, Subcommand)]
pub(crate) enum McpCommand {
    /// Serve jev's tools to an MCP client over stdio
    Serve(Pending),
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::{Cli, Command};
    use crate::output::Format;

    #[test]
    fn the_command_tree_is_internally_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn the_full_surface_of_the_prd_is_present() {
        let command = Cli::command();
        let mut paths: Vec<String> = Vec::new();
        for top in command.get_subcommands().filter(|sub| !sub.is_hide_set()) {
            if top.get_subcommands().next().is_none() {
                paths.push(top.get_name().to_owned());
            }
            for leaf in top.get_subcommands() {
                paths.push(format!("{} {}", top.get_name(), leaf.get_name()));
            }
        }

        let expected = [
            "eval",
            "noul",
            "choice",
            "score",
            "validate",
            "batch run",
            "models list",
            "auth login",
            "auth status",
            "auth logout",
            "config get",
            "config set",
            "config unset",
            "config list",
            "config path",
            "profile list",
            "profile use",
            "profile create",
            "profile delete",
            "schema request",
            "schema questions",
            "schema batch-record",
            "schema output",
            "schema error",
            "spec",
            "mcp serve",
            "update",
            "completion",
            "version",
        ];
        assert_eq!(paths, expected);
    }

    #[test]
    fn every_command_and_global_flag_has_help_text() {
        fn check(command: &clap::Command, path: &str) {
            assert!(command.get_about().is_some(), "`{path}` has no description");
            for argument in command
                .get_arguments()
                .filter(|argument| !argument.is_hide_set())
            {
                assert!(
                    argument.get_help().is_some(),
                    "`{path}` flag `{}` has no help",
                    argument.get_id()
                );
            }
            for sub in command.get_subcommands() {
                check(sub, &format!("{path} {}", sub.get_name()));
            }
        }
        check(&Cli::command(), "jev");
    }

    #[test]
    fn global_flags_work_before_and_after_the_command() {
        let before =
            Cli::try_parse_from(["jev", "-o", "json", "--field", "version", "version"]).unwrap();
        let after =
            Cli::try_parse_from(["jev", "version", "--output", "YAML", "-vv", "--no-color"])
                .unwrap();

        assert_eq!(before.global.output, Some(Format::Json));
        assert_eq!(before.global.field.as_deref(), Some("version"));
        assert_eq!(after.global.output, Some(Format::Yaml));
        assert_eq!(after.global.verbose, 2);
        assert!(after.global.no_color);
        assert!(matches!(after.command, Command::Version));
    }

    #[test]
    fn a_pending_command_accepts_whatever_follows_it() {
        let cli =
            Cli::try_parse_from(["jev", "noul", "Is it urgent?", "--fail-under", "0.7", "-x"])
                .unwrap();

        let Command::Noul(pending) = cli.command else {
            panic!("expected noul")
        };
        assert_eq!(pending.rest.len(), 4);
    }

    #[test]
    fn quiet_and_verbose_cannot_be_combined() {
        assert!(Cli::try_parse_from(["jev", "-q", "-v", "version"]).is_err());
    }
}
