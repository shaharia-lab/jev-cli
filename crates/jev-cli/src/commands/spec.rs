//! `jev spec`: the whole command tree as one JSON document, for agents and tooling.
//!
//! Everything comes from the tree `jev` parses with, help attached ([`crate::cli::command`]), and
//! from the settings registry for defaults and environment variables, so that it cannot say
//! something `--help` does not. Nothing is read from the network or the configuration.

use std::any::TypeId;
use std::time::Duration;

use clap::{Arg, ArgAction};
use serde::Serialize;
use serde_json::Value;

use super::Context;
use crate::config::Key;
use crate::error::CliError;
use crate::exit::Exit;
use crate::help::{self, Example, ExitCode};
use crate::output::{Render, Ui};

/// The version of the shape of this document. It changes only when a field is removed or changes
/// meaning; new fields may appear at any time.
const SPEC_VERSION: u32 = 1;

pub(crate) fn run(context: &mut Context<'_>) -> Result<(), CliError> {
    context.output.emit(&CommandTree::new(), context.stdout)
}

/// The command tree. This shape is versioned public API.
///
/// The fields are `pub(crate)` because [`crate::reference`] renders the Markdown command
/// reference from this same document, so that the reference cannot say something `jev spec` and
/// `--help` do not.
#[derive(Debug, Serialize)]
pub(crate) struct CommandTree {
    spec_version: u32,
    pub(crate) name: String,
    /// The version of `jev` this describes.
    version: &'static str,
    pub(crate) about: String,
    pub(crate) description: String,
    /// Flags every command accepts, before or after the command's name.
    pub(crate) global_flags: Vec<Flag>,
    /// The whole exit-code contract.
    pub(crate) exit_codes: Vec<ExitCodeSpec>,
    pub(crate) examples: &'static [Example],
    /// Every command that can be run, in the order of `jev --help`.
    pub(crate) commands: Vec<CommandSpec>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CommandSpec {
    /// The words after `jev`, such as `auth login`.
    pub(crate) path: String,
    pub(crate) usage: String,
    pub(crate) about: String,
    /// `false` for a command that is in the tree but answers "not implemented" for now.
    pub(crate) implemented: bool,
    pub(crate) when_to_use: Option<&'static str>,
    pub(crate) input: Option<&'static str>,
    pub(crate) output: Option<&'static str>,
    pub(crate) arguments: Vec<Argument>,
    /// The command's own flags; `global_flags` apply as well.
    pub(crate) flags: Vec<Flag>,
    pub(crate) exit_codes: Vec<ExitCodeSpec>,
    pub(crate) examples: &'static [Example],
}

#[derive(Debug, Serialize)]
pub(crate) struct Argument {
    pub(crate) name: String,
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) required: bool,
    pub(crate) multiple: bool,
    pub(crate) help: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct Flag {
    /// The long form, such as `--state-file`.
    pub(crate) name: String,
    pub(crate) short: Option<String>,
    /// What the value is called in the help, such as `PATH`. `null` for a switch.
    pub(crate) value_name: Option<String>,
    /// `boolean`, `count`, `enum`, `integer`, `number`, `duration` or `string`.
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) required: bool,
    /// Whether the flag may be repeated.
    pub(crate) multiple: bool,
    pub(crate) default: Option<Value>,
    /// The accepted values of an `enum`.
    pub(crate) values: Option<Vec<PossibleValue>>,
    /// The environment variable that supplies the value when the flag is not given.
    pub(crate) env: Option<String>,
    /// The setting (`jev config`) the flag overrides for one run.
    pub(crate) setting: Option<&'static str>,
    pub(crate) heading: Option<String>,
    pub(crate) help: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct PossibleValue {
    pub(crate) value: String,
    help: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ExitCodeSpec {
    pub(crate) code: u8,
    pub(crate) name: &'static str,
    pub(crate) meaning: &'static str,
}

impl ExitCodeSpec {
    fn new(exit_code: &ExitCode) -> Self {
        Self {
            code: exit_code.exit.code(),
            name: exit_code.exit.name(),
            meaning: exit_code.meaning(),
        }
    }
}

impl CommandTree {
    pub(crate) fn new() -> Self {
        let mut root = crate::cli::command();
        // Propagates the global flags and the full `jev ...` name into every subcommand.
        root.build();

        let mut commands = Vec::new();
        collect(&root, "", &mut commands);
        Self {
            spec_version: SPEC_VERSION,
            name: root.get_name().to_owned(),
            version: env!("CARGO_PKG_VERSION"),
            about: text(root.get_about()),
            description: text(root.get_long_about()),
            global_flags: root
                .get_arguments()
                .filter(|argument| argument.is_global_set() && shown(argument))
                .map(Flag::new)
                .collect(),
            exit_codes: Exit::ALL
                .into_iter()
                .map(|exit| ExitCodeSpec {
                    code: exit.code(),
                    name: exit.name(),
                    meaning: exit.meaning(),
                })
                .collect(),
            examples: help::ROOT_EXAMPLES,
            commands,
        }
    }
}

/// Every visible command that runs something, below `command`.
fn collect(command: &clap::Command, path: &str, commands: &mut Vec<CommandSpec>) {
    for sub in command.get_subcommands().filter(|sub| !sub.is_hide_set()) {
        let below = help::join(path, sub.get_name());
        if sub.has_subcommands() {
            collect(sub, &below, commands);
        } else {
            commands.push(CommandSpec::new(sub, below));
        }
    }
}

impl CommandSpec {
    fn new(command: &clap::Command, path: String) -> Self {
        let doc = help::find(&path);
        let usage = command.clone().render_usage().to_string();
        let own = || {
            command
                .get_arguments()
                .filter(|argument| !argument.is_global_set() && shown(argument))
        };
        Self {
            usage: usage.trim_start_matches("Usage:").trim().to_owned(),
            about: text(command.get_about()),
            implemented: !help::is_pending(command),
            when_to_use: doc.map(|doc| doc.when),
            input: doc.map(|doc| doc.input),
            output: doc.map(|doc| doc.output),
            arguments: own()
                .filter(|argument| argument.is_positional())
                .map(Argument::new)
                .collect(),
            flags: own()
                .filter(|argument| !argument.is_positional())
                .map(Flag::new)
                .collect(),
            exit_codes: doc
                .map(|doc| doc.exit_codes.iter().map(ExitCodeSpec::new).collect())
                .unwrap_or_default(),
            examples: doc.map_or(&[], |doc| doc.examples),
            path,
        }
    }
}

impl Argument {
    fn new(argument: &Arg) -> Self {
        Self {
            name: value_name(argument).unwrap_or_else(|| argument.get_id().to_string()),
            kind: kind(argument),
            required: argument.is_required_set(),
            multiple: multiple(argument),
            help: help_text(argument),
        }
    }
}

impl Flag {
    fn new(argument: &Arg) -> Self {
        let name = argument
            .get_long()
            .map_or_else(|| argument.get_id().to_string(), |long| format!("--{long}"));
        let kind = kind(argument);
        let setting = Key::ALL
            .into_iter()
            .find(|key| key.flag() == Some(name.as_str()));
        let values: Vec<PossibleValue> = argument
            .get_possible_values()
            .iter()
            .filter(|value| !value.is_hide_set())
            .map(|value| PossibleValue {
                value: value.get_name().to_owned(),
                help: value.get_help().map(ToString::to_string),
            })
            .collect();
        Self {
            short: argument.get_short().map(|short| format!("-{short}")),
            value_name: if takes_value(argument) {
                value_name(argument)
            } else {
                None
            },
            kind,
            required: argument.is_required_set(),
            multiple: multiple(argument),
            default: setting.map_or_else(
                || clap_default(argument, kind),
                |key| {
                    key.default_value()
                        .and_then(|value| serde_json::to_value(value).ok())
                },
            ),
            values: (kind == "enum").then_some(values),
            env: argument
                .get_env()
                .map(|variable| variable.to_string_lossy().into_owned())
                .or_else(|| setting.and_then(Key::env_var).map(str::to_owned))
                // The profile is chosen before settings are resolved, so it is not a `Key`; see
                // `Settings::resolve`.
                .or_else(|| (name == "--profile").then(|| "JEV_PROFILE".to_owned())),
            setting: setting.map(Key::name),
            heading: argument.get_help_heading().map(str::to_owned),
            help: help_text(argument),
            name,
        }
    }
}

/// Whether an argument belongs in the spec: not hidden, and not clap's own `--help`/`--version`.
fn shown(argument: &Arg) -> bool {
    !argument.is_hide_set()
        && !matches!(
            argument.get_action(),
            ArgAction::Help | ArgAction::HelpShort | ArgAction::HelpLong | ArgAction::Version
        )
}

fn takes_value(argument: &Arg) -> bool {
    matches!(argument.get_action(), ArgAction::Set | ArgAction::Append)
}

fn multiple(argument: &Arg) -> bool {
    matches!(argument.get_action(), ArgAction::Append)
        || argument
            .get_num_args()
            .is_some_and(|range| range.max_values() > 1)
}

/// The type of the value, as far as a caller needs to know it to build a command line.
fn kind(argument: &Arg) -> &'static str {
    match argument.get_action() {
        ArgAction::SetTrue | ArgAction::SetFalse => return "boolean",
        ArgAction::Count => return "count",
        _ => {}
    }
    if !argument.get_possible_values().is_empty() {
        return "enum";
    }
    let parsed = argument.get_value_parser().type_id();
    if parsed == TypeId::of::<f64>() {
        "number"
    } else if [
        TypeId::of::<u8>(),
        TypeId::of::<u16>(),
        TypeId::of::<u32>(),
        TypeId::of::<u64>(),
        TypeId::of::<usize>(),
        TypeId::of::<i64>(),
    ]
    .into_iter()
    .any(|integer| parsed == integer)
    {
        "integer"
    } else if parsed == TypeId::of::<Duration>() {
        "duration"
    } else {
        "string"
    }
}

/// The default `clap` fills in, typed like the value.
fn clap_default(argument: &Arg, kind: &str) -> Option<Value> {
    let default = argument.get_default_values().first()?.to_string_lossy();
    Some(match kind {
        "boolean" => Value::Bool(default == "true"),
        "count" | "integer" | "number" => serde_json::from_str(&default).ok()?,
        _ => Value::String(default.into_owned()),
    })
}

fn value_name(argument: &Arg) -> Option<String> {
    argument
        .get_value_names()
        .and_then(<[_]>::first)
        .map(ToString::to_string)
}

fn help_text(argument: &Arg) -> String {
    text(argument.get_long_help().or_else(|| argument.get_help()))
}

fn text(styled: Option<&clap::builder::StyledStr>) -> String {
    styled.map(ToString::to_string).unwrap_or_default()
}

impl Render for CommandTree {
    /// JSON on a terminal too: the spec is for programs, and a person can read JSON.
    fn human(&self, _: Ui) -> String {
        serde_json::to_string_pretty(self)
            .map(|json| format!("{json}\n"))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::CommandTree;

    fn spec() -> Value {
        serde_json::to_value(CommandTree::new()).unwrap()
    }

    fn command<'a>(spec: &'a Value, path: &str) -> &'a Value {
        spec["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["path"] == path)
            .unwrap_or_else(|| panic!("no `{path}` in the spec"))
    }

    fn flag<'a>(flags: &'a Value, name: &str) -> &'a Value {
        flags
            .as_array()
            .unwrap()
            .iter()
            .find(|flag| flag["name"] == name)
            .unwrap_or_else(|| panic!("no `{name}` in {flags}"))
    }

    #[test]
    fn global_flags_carry_their_type_default_and_environment_variable() {
        let spec = spec();
        let globals = &spec["global_flags"];

        let timeout = flag(globals, "--timeout");
        assert_eq!(timeout["type"], "duration");
        assert_eq!(timeout["default"], "30s");
        assert_eq!(timeout["setting"], "timeout");

        let model = flag(globals, "--model");
        assert_eq!(model["env"], "TYPESAFE_DEFAULT_MODEL");
        assert_eq!(model["default"], "jev-latest");

        let output = flag(globals, "--output");
        assert_eq!(output["type"], "enum");
        assert_eq!(output["short"], "-o");
        assert_eq!(output["env"], "JEV_OUTPUT");
        let values: Vec<&str> = output["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value["value"].as_str().unwrap())
            .collect();
        assert_eq!(values, ["table", "json", "yaml", "jsonl"]);

        assert_eq!(flag(globals, "--max-retries")["type"], "integer");
        assert_eq!(flag(globals, "--max-retries")["default"], 2);
        assert_eq!(flag(globals, "--no-input")["type"], "boolean");
        assert_eq!(flag(globals, "--no-input")["env"], "JEV_NO_INPUT");
        assert_eq!(flag(globals, "--verbose")["type"], "count");
        assert_eq!(flag(globals, "--profile")["env"], "JEV_PROFILE");
        assert!(
            globals
                .as_array()
                .unwrap()
                .iter()
                .all(|flag| flag["name"] != "--help" && flag["name"] != "--version"),
            "{globals}"
        );
    }

    #[test]
    fn a_command_has_its_own_flags_arguments_and_doc() {
        let spec = spec();
        let noul = command(&spec, "noul");

        assert_eq!(noul["implemented"], true);
        assert!(noul["usage"].as_str().unwrap().starts_with("jev noul"));
        assert!(
            noul["when_to_use"]
                .as_str()
                .unwrap()
                .contains("`jev choice`")
        );
        assert_eq!(noul["arguments"][0]["name"], "QUESTION");
        assert_eq!(flag(&noul["flags"], "--fail-under")["type"], "number");
        assert_eq!(flag(&noul["flags"], "--fail-under")["heading"], "Gating");
        assert_eq!(flag(&noul["flags"], "--state-format")["default"], "auto");
        assert_eq!(flag(&noul["flags"], "--dry-run")["default"], false);
        assert!(
            noul["flags"]
                .as_array()
                .unwrap()
                .iter()
                .all(|flag| flag["name"] != "--output"),
            "global flags are listed once, under global_flags"
        );
        let codes: Vec<u64> = noul["exit_codes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|code| code["code"].as_u64().unwrap())
            .collect();
        assert_eq!(codes, [0, 2, 3, 4, 5, 6, 10, 11]);
        assert!(noul["examples"].as_array().unwrap().len() >= 2);

        assert_eq!(
            flag(&command(&spec, "eval")["flags"], "--file")["required"],
            true
        );
        assert_eq!(
            flag(&command(&spec, "choice")["flags"], "--option")["multiple"],
            true
        );
    }

    #[test]
    fn hidden_commands_are_left_out() {
        let spec = spec();

        assert!(
            spec["commands"]
                .as_array()
                .unwrap()
                .iter()
                .all(|command| !command["path"].as_str().unwrap().starts_with("debug")),
        );
    }
}
