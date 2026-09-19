//! The command tree. `clap` is the single source for parsing, `--help` and `jev spec`.
//!
//! Each command's one-line purpose and its arguments are here; the rest of its help (when to use
//! it, input, output, exit codes, examples) is its `Doc` in `help/docs.rs`, attached by [`command`].

use std::ffi::OsString;
use std::time::Duration;

use clap::builder::FalseyValueParser;
use clap::{ArgAction, Args, CommandFactory, Parser, Subcommand};

use crate::batch::RowFormat;
use crate::gate::AbstainBand;
use crate::input::{InputFormat, StateFormat};
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
Exit codes are a stable contract (listed under Exit codes).\n\n\
Agents and tooling: `jev spec` prints every command, flag, exit code and example as JSON, and \
`jev <command> --help` says when to use each command.\n\n\
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

/// The command tree with its help: what `jev` parses, and what `--help` and `jev spec` show.
pub(crate) fn command() -> clap::Command {
    crate::help::apply(Cli::command())
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

    /// Allow a plain http:// base URL to a host other than localhost (the API key travels unencrypted)
    #[arg(long, global = true)]
    pub(crate) insecure_allow_http: bool,

    /// Log request and response bodies with -v; `state` may contain sensitive data
    #[arg(long, global = true)]
    pub(crate) debug_bodies: bool,

    /// Print errors only; suppress notices and warnings on stderr
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    pub(crate) quiet: bool,

    /// Log to stderr; repeat for more (-v requests and retries, -vv everything)
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub(crate) verbose: u8,
}

/// Where the state comes from, when it is not the request file. Shared by every evaluating command.
#[derive(Debug, Default, Args)]
#[command(next_help_heading = "State")]
pub(crate) struct StateArgs {
    /// The state to evaluate, as text
    #[arg(long, value_name = "TEXT", conflicts_with = "state_file")]
    pub(crate) state: Option<String>,

    /// Read the state from a file, or from stdin with `-`
    #[arg(long, value_name = "PATH")]
    pub(crate) state_file: Option<String>,

    /// How to read the state: `json` sends an object or array, `text` one string
    #[arg(long, value_enum, default_value_t, value_name = "FORMAT")]
    pub(crate) state_format: StateFormat,
}

/// How the request is checked and sent. Shared by every evaluating command.
// On/off command-line switches are booleans by nature; there is no state machine hiding here.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default, Args)]
#[command(next_help_heading = "Sending")]
pub(crate) struct SendArgs {
    /// Validate, print the exact request body and a size estimate, and send nothing
    #[arg(long)]
    pub(crate) dry_run: bool,

    /// Treat validation warnings as errors
    #[arg(long)]
    pub(crate) strict: bool,

    /// Skip the offline estimate of the request's size
    #[arg(long)]
    pub(crate) skip_size_check: bool,

    /// Remind me on stderr to pin a versioned model id when the request used an alias
    #[arg(long)]
    pub(crate) warn_unpinned: bool,

    /// Print the API response body exactly as received, ignoring --output
    #[arg(long, conflicts_with_all = ["field", "dry_run"])]
    pub(crate) raw: bool,
}

/// Arguments of `jev eval`.
#[derive(Debug, Args)]
pub(crate) struct EvalArgs {
    /// Request file, JSON or YAML, with `questions` and optionally `state` and `model`; `-` reads stdin
    #[arg(short = 'f', long, value_name = "FILE")]
    pub(crate) file: String,

    /// Format of the request file [default: by extension; for stdin, JSON if it starts with `{`]
    #[arg(long, value_enum, value_name = "FORMAT")]
    pub(crate) input_format: Option<InputFormat>,

    /// Exit 10 unless this holds, e.g. `is_urgent >= 0.7`, `team == billing`, `team.confidence >= 0.8`
    #[arg(long = "assert", value_name = "CONDITION", help_heading = "Gating")]
    pub(crate) assertions: Vec<String>,

    #[command(flatten)]
    pub(crate) state: StateArgs,

    #[command(flatten)]
    pub(crate) send: SendArgs,
}

/// What a shortcut command asks, when it is not given as plain text.
#[derive(Debug, Default, Args)]
pub(crate) struct QuestionArgs {
    /// The question, in plain words
    #[arg(
        value_name = "QUESTION",
        required_unless_present = "instructions_file",
        conflicts_with = "instructions_file"
    )]
    pub(crate) question: Option<String>,

    /// Structured instructions (JSON or YAML: `what`, `not_for`, `examples`, ...) instead of QUESTION
    #[arg(long, value_name = "FILE")]
    pub(crate) instructions_file: Option<String>,
}

/// Arguments of `jev noul`.
#[derive(Debug, Args)]
pub(crate) struct NoulArgs {
    #[command(flatten)]
    pub(crate) question: QuestionArgs,

    /// What a yes (a value near 1) means
    #[arg(long = "true", value_name = "TEXT", conflicts_with = "criteria_file")]
    pub(crate) when_true: Option<String>,

    /// What a no (a value near 0) means
    #[arg(long = "false", value_name = "TEXT", conflicts_with = "criteria_file")]
    pub(crate) when_false: Option<String>,

    /// Structured criteria (JSON or YAML) with `true` and `false`
    #[arg(long, value_name = "FILE")]
    pub(crate) criteria_file: Option<String>,

    /// Exit 10 when P(yes) is below this
    #[arg(long, value_name = "P", help_heading = "Gating", value_parser = parse_probability)]
    pub(crate) fail_under: Option<f64>,

    /// Exit 10 when P(yes) is above this
    #[arg(long, value_name = "P", help_heading = "Gating", value_parser = parse_probability)]
    pub(crate) fail_over: Option<f64>,

    /// Exit 11 when P(yes) is inside `LO,HI`, e.g. `0.4,0.6`: near 0.5 the model cannot tell
    #[arg(long, value_name = "LO,HI", help_heading = "Gating", value_parser = AbstainBand::parse)]
    pub(crate) abstain_band: Option<AbstainBand>,

    #[command(flatten)]
    pub(crate) state: StateArgs,

    #[command(flatten)]
    pub(crate) send: SendArgs,
}

/// Arguments of `jev choice`.
#[derive(Debug, Args)]
pub(crate) struct ChoiceArgs {
    #[command(flatten)]
    pub(crate) question: QuestionArgs,

    /// An option, as `name` or `name=description`; repeat for each (up to 255). Include a way out such as `other`
    #[arg(
        long = "option",
        value_name = "NAME[=DESCRIPTION]",
        conflicts_with = "criteria_file"
    )]
    pub(crate) options: Vec<String>,

    /// Structured criteria (JSON or YAML): a map of option name to description
    #[arg(long, value_name = "FILE")]
    pub(crate) criteria_file: Option<String>,

    /// Do not warn when no option offers a way out (`other`, `none_of_the_above`, `not_stated`, ...)
    #[arg(long)]
    pub(crate) no_escape_warning: bool,

    /// Exit 10 unless this option wins; repeat to accept any of several
    #[arg(long, value_name = "OPTION", help_heading = "Gating")]
    pub(crate) expect: Vec<String>,

    /// Exit 10 when the confidence is below this
    #[arg(long, value_name = "C", help_heading = "Gating", value_parser = parse_probability)]
    pub(crate) min_confidence: Option<f64>,

    #[command(flatten)]
    pub(crate) state: StateArgs,

    #[command(flatten)]
    pub(crate) send: SendArgs,
}

/// Arguments of `jev score`.
#[derive(Debug, Args)]
pub(crate) struct ScoreArgs {
    #[command(flatten)]
    pub(crate) question: QuestionArgs,

    /// A level of the rubric, lowest first; repeat for each (2 to 10)
    #[arg(
        long = "level",
        value_name = "DESCRIPTION",
        conflicts_with = "criteria_file"
    )]
    pub(crate) levels: Vec<String>,

    /// Structured criteria (JSON or YAML): an ordered list of level descriptions
    #[arg(long, value_name = "FILE")]
    pub(crate) criteria_file: Option<String>,

    /// Exit 10 when the score is below this
    #[arg(long, value_name = "SCORE", help_heading = "Gating")]
    pub(crate) fail_under: Option<f64>,

    /// Exit 10 when the score is above this
    #[arg(long, value_name = "SCORE", help_heading = "Gating")]
    pub(crate) fail_over: Option<f64>,

    /// Exit 10 when the confidence is below this
    #[arg(long, value_name = "C", help_heading = "Gating", value_parser = parse_probability)]
    pub(crate) min_confidence: Option<f64>,

    #[command(flatten)]
    pub(crate) state: StateArgs,

    #[command(flatten)]
    pub(crate) send: SendArgs,
}

/// A probability, or a confidence: a number from 0 to 1.
fn parse_probability(text: &str) -> Result<f64, String> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|value| (0.0..=1.0).contains(value))
        .ok_or_else(|| format!("`{text}` is not a number from 0 to 1"))
}

/// Arguments of `jev validate`.
#[derive(Debug, Args)]
pub(crate) struct ValidateArgs {
    /// Request file, JSON or YAML; `state` and `model` may be left out; `-` reads stdin
    #[arg(short = 'f', long, value_name = "FILE")]
    pub(crate) file: String,

    /// Format of the request file [default: by extension; for stdin, JSON if it starts with `{`]
    #[arg(long, value_enum, value_name = "FORMAT")]
    pub(crate) input_format: Option<InputFormat>,

    /// Check the request with this state, as text, so that its size can be estimated
    #[arg(long, value_name = "TEXT", conflicts_with = "state_file")]
    pub(crate) state: Option<String>,

    /// Check the request with the state in this file, or on stdin with `-`
    #[arg(long, value_name = "PATH")]
    pub(crate) state_file: Option<String>,

    /// How to read a state given outside the request file
    #[arg(long, value_enum, default_value_t, value_name = "FORMAT")]
    pub(crate) state_format: StateFormat,

    /// Count warnings as errors, so that any finding makes the request invalid
    #[arg(long)]
    pub(crate) strict: bool,

    /// Skip the offline estimate of the request's size
    #[arg(long)]
    pub(crate) skip_size_check: bool,
}

/// Arguments of `jev auth login`.
#[derive(Debug, Args)]
pub(crate) struct LoginArgs {
    /// Read the key from stdin instead of prompting (for scripts and agents)
    #[arg(long)]
    pub(crate) with_token: bool,

    /// Store the key without checking it against the API first
    #[arg(long)]
    pub(crate) skip_verify: bool,
}

/// Arguments of `jev auth status`.
#[derive(Debug, Args)]
pub(crate) struct StatusArgs {
    /// Do not check the key against the API
    #[arg(long)]
    pub(crate) offline: bool,
}

/// Arguments of `jev auth logout`.
#[derive(Debug, Args)]
pub(crate) struct LogoutArgs {
    /// Remove the stored keys of every profile, not only the selected one
    #[arg(long)]
    pub(crate) all: bool,
}

/// Arguments of `jev batch run`.
// On/off command-line switches are booleans by nature; there is no state machine hiding here.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Args)]
pub(crate) struct BatchRunArgs {
    /// Question set: a request file (JSON or YAML) with `questions` and optionally `model`
    #[arg(short = 'f', long, value_name = "FILE")]
    pub(crate) file: String,

    /// Rows to evaluate: a JSONL or CSV file, or `-` for JSONL on stdin [default: stdin when piped]
    #[arg(long, value_name = "PATH", help_heading = "Rows")]
    pub(crate) input: Option<String>,

    /// Format of --input [default: csv for a `.csv` file, jsonl otherwise]
    #[arg(long, value_enum, value_name = "FORMAT", help_heading = "Rows")]
    pub(crate) input_format: Option<RowFormat>,

    /// Send this one field of each row as its state [default: the whole row]
    #[arg(
        long,
        value_name = "NAME",
        conflicts_with = "state_fields",
        help_heading = "Rows"
    )]
    pub(crate) state_field: Option<String>,

    /// Send an object of only these fields as the state, e.g. `subject,body`
    #[arg(
        long,
        value_name = "NAME,...",
        value_delimiter = ',',
        help_heading = "Rows"
    )]
    pub(crate) state_fields: Vec<String>,

    /// Field that identifies each row in the results; must be unique [default: the line number]
    #[arg(long, value_name = "NAME", help_heading = "Rows")]
    pub(crate) id_field: Option<String>,

    /// Write the records to this file instead of stdout; it must not exist yet, or be empty
    #[arg(long, value_name = "PATH", help_heading = "Results")]
    pub(crate) out: Option<String>,

    /// Also write the end-of-run summary to this file, as one JSON object
    #[arg(long, value_name = "PATH", help_heading = "Results")]
    pub(crate) summary_json: Option<String>,

    /// Requests in flight at once, 1 to 64; shared keys get rate limited above about 8 [default: 4]
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=64))]
    pub(crate) concurrency: Option<u32>,

    /// Stop sending at the first row that fails; rows in flight still finish
    #[arg(long, conflicts_with = "max_errors")]
    pub(crate) fail_fast: bool,

    /// Stop sending once N rows have failed; rows in flight still finish
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u64).range(1..))]
    pub(crate) max_errors: Option<u64>,

    /// Treat validation warnings as errors
    #[arg(long, help_heading = "Sending")]
    pub(crate) strict: bool,

    /// Skip the offline estimate of each request's size
    #[arg(long, help_heading = "Sending")]
    pub(crate) skip_size_check: bool,

    /// Remind me on stderr to pin a versioned model id when the run used an alias
    #[arg(long, help_heading = "Sending")]
    pub(crate) warn_unpinned: bool,
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
    Eval(EvalArgs),
    /// Ask one yes/no question; returns the probability of yes
    Noul(NoulArgs),
    /// Pick one option from a set you define
    Choice(ChoiceArgs),
    /// Rate the state on an ordered rubric of 2 to 10 levels
    Score(ScoreArgs),
    /// Check a request file offline; nothing is sent or billed
    Validate(ValidateArgs),
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
    Spec,
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

impl Command {
    /// The value of `--concurrency`, which only `jev batch run` has, for the settings resolver.
    pub(crate) const fn concurrency(&self) -> Option<u32> {
        match self {
            Self::Batch(BatchCommand::Run(arguments)) => arguments.concurrency,
            _ => None,
        }
    }
}

#[derive(Debug, Subcommand)]
pub(crate) enum BatchCommand {
    /// Evaluate every row of an input file, writing one result record per row
    Run(BatchRunArgs),
}

#[derive(Debug, Subcommand)]
pub(crate) enum ModelsCommand {
    /// List model names and aliases, with their description and release date
    List,
}

#[derive(Debug, Subcommand)]
pub(crate) enum AuthCommand {
    /// Store an API key for the profile, once, after checking it against the API
    Login(LoginArgs),
    /// Show whether a key is configured, where it comes from, and whether the API accepts it
    Status(StatusArgs),
    /// Remove the stored API key
    Logout(LogoutArgs),
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
    /// JSON Schema of a complete request: `state`, `model` and `questions`, as the API takes it
    Request,
    /// JSON Schema of a question set: a request file in which only `questions` is required
    Questions,
    /// JSON Schema of one line of `jev batch run` output
    BatchRecord,
    /// JSON Schema of the result that `jev eval`, `jev noul`, `jev choice` and `jev score` print
    Output,
    /// JSON Schema of the error object printed on stderr for a program
    Error,
}

/// Arguments of `jev mcp serve`.
#[derive(Debug, Args)]
pub(crate) struct McpServeArgs {
    /// Refuse, before sending, any call whose estimated cost in US dollars is above this
    #[arg(long, value_name = "USD", value_parser = parse_usd, help_heading = "Guardrails")]
    pub(crate) max_cost_usd_per_call: Option<f64>,
}

/// An amount of US dollars: a finite number, zero or more.
fn parse_usd(text: &str) -> Result<f64, String> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| format!("`{text}` is not an amount of US dollars, such as 0.01"))
}

#[derive(Debug, Subcommand)]
pub(crate) enum McpCommand {
    /// Serve jev's tools to an MCP client (Claude Code, Claude Desktop, Cursor, VS Code) over stdio
    Serve(McpServeArgs),
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
        let cli = Cli::try_parse_from(["jev", "update", "--format", "json", "-x"]).unwrap();

        let Command::Update(pending) = cli.command else {
            panic!("expected update")
        };
        assert_eq!(pending.rest.len(), 3);
    }

    #[test]
    fn a_shortcut_needs_a_question_or_an_instructions_file_but_not_both() {
        assert!(Cli::try_parse_from(["jev", "noul", "Is it urgent?"]).is_ok());
        assert!(Cli::try_parse_from(["jev", "noul", "--instructions-file", "i.yaml"]).is_ok());
        assert!(Cli::try_parse_from(["jev", "noul"]).is_err());
        assert!(
            Cli::try_parse_from(["jev", "noul", "Is it?", "--instructions-file", "i.yaml"])
                .is_err()
        );
        assert!(
            Cli::try_parse_from(["jev", "noul", "Is it?", "--fail-under", "1.5"]).is_err(),
            "a probability is 0 to 1"
        );
        assert!(
            Cli::try_parse_from(["jev", "noul", "Is it?", "--abstain-band", "0.6,0.4"]).is_err()
        );
        assert!(
            Cli::try_parse_from([
                "jev",
                "score",
                "How?",
                "--level",
                "a",
                "--criteria-file",
                "c.yaml"
            ])
            .is_err()
        );
    }

    #[test]
    fn quiet_and_verbose_cannot_be_combined() {
        assert!(Cli::try_parse_from(["jev", "-q", "-v", "version"]).is_err());
    }
}
