//! `jev noul`, `jev choice` and `jev score`: one question from the command line.
//!
//! Each builds the same request document `jev eval` reads from a file, and then runs it the same
//! way, so validation, model resolution and the state inputs cannot differ between them.

use jev_client::validate::Document;
use serde::Serialize;
use serde_json::{Map, Value, json};

use super::Context;
use super::eval::{Plan, Source, evaluate_plan};
use crate::cli::{ChoiceArgs, NoulArgs, QuestionArgs, ScoreArgs, SendArgs, StateArgs};
use crate::error::CliError;
use crate::exit::Exit;
use crate::gate::{self, AbstainBand, Condition, GateOutcome, Op};
use crate::input;
use crate::output::{Render, ResultEnvelope, Ui};

/// The id of a shortcut's only question. It is never sent to the model, and never shown.
const ANSWER: &str = "answer";

pub(crate) fn noul(arguments: &NoulArgs, context: &mut Context<'_>) -> Result<Exit, CliError> {
    let criteria = if let Some(path) = &arguments.criteria_file {
        Some(structured(path, "--criteria-file", context)?)
    } else if arguments.when_true.is_some() || arguments.when_false.is_some() {
        let sides = [
            ("true", &arguments.when_true),
            ("false", &arguments.when_false),
        ];
        let described = sides
            .into_iter()
            .filter_map(|(side, text)| Some((side.to_owned(), json!(text.as_ref()?))));
        Some(Value::Object(described.collect()))
    } else {
        None
    };
    let mut question = Map::new();
    question.insert("type".to_owned(), json!("noul"));
    question.insert(
        "instructions".to_owned(),
        instructions(&arguments.question, context)?,
    );
    if let Some(criteria) = criteria {
        question.insert("criteria".to_owned(), criteria);
    }
    let question = Value::Object(question);

    let mut conditions = Vec::new();
    if let Some(threshold) = arguments.fail_under {
        conditions.push(Condition::number(ANSWER, false, Op::Ge, threshold));
    }
    if let Some(threshold) = arguments.fail_over {
        conditions.push(Condition::number(ANSWER, false, Op::Le, threshold));
    }
    let shortcut = Shortcut {
        question,
        conditions,
        abstain: arguments.abstain_band,
        hidden_lints: &[],
    };
    shortcut.run(&arguments.state, &arguments.send, context)
}

pub(crate) fn choice(arguments: &ChoiceArgs, context: &mut Context<'_>) -> Result<Exit, CliError> {
    let criteria = match &arguments.criteria_file {
        Some(path) => structured(path, "--criteria-file", context)?,
        None => options_from_flags(&arguments.options)?,
    };
    let question = json!({ "type": "choice", "instructions": instructions(&arguments.question, context)?, "criteria": criteria });

    let mut conditions = Vec::new();
    if !arguments.expect.is_empty() {
        conditions.push(Condition::any_of(ANSWER, arguments.expect.clone()));
    }
    if let Some(confidence) = arguments.min_confidence {
        conditions.push(Condition::number(ANSWER, true, Op::Ge, confidence));
    }
    let hidden: &[&str] = if arguments.no_escape_warning {
        &["choice-no-escape-option"]
    } else {
        &[]
    };
    Shortcut {
        question,
        conditions,
        abstain: None,
        hidden_lints: hidden,
    }
    .run(&arguments.state, &arguments.send, context)
}

pub(crate) fn score(arguments: &ScoreArgs, context: &mut Context<'_>) -> Result<Exit, CliError> {
    let criteria = match &arguments.criteria_file {
        Some(path) => structured(path, "--criteria-file", context)?,
        None => json!(arguments.levels),
    };
    let question = json!({ "type": "score", "instructions": instructions(&arguments.question, context)?, "criteria": criteria });

    let mut conditions = Vec::new();
    if let Some(threshold) = arguments.fail_under {
        conditions.push(Condition::number(ANSWER, false, Op::Ge, threshold));
    }
    if let Some(threshold) = arguments.fail_over {
        conditions.push(Condition::number(ANSWER, false, Op::Le, threshold));
    }
    if let Some(confidence) = arguments.min_confidence {
        conditions.push(Condition::number(ANSWER, true, Op::Ge, confidence));
    }
    Shortcut {
        question,
        conditions,
        abstain: None,
        hidden_lints: &[],
    }
    .run(&arguments.state, &arguments.send, context)
}

/// Turns repeated `--option name[=description]` flags into choice criteria, in the order given.
/// An option without a description is sent as `null`.
fn options_from_flags(flags: &[String]) -> Result<Value, CliError> {
    let mut options = Map::new();
    for flag in flags {
        let (name, description) = flag
            .split_once('=')
            .map_or((flag.as_str(), Value::Null), |(name, text)| {
                (name, json!(text))
            });
        let name = name.trim();
        if name.is_empty() {
            return Err(CliError::usage(format!("--option `{flag}` has no name"))
                .hint("write --option name or --option name=description"));
        }
        if options.insert(name.to_owned(), description).is_some() {
            return Err(
                CliError::usage(format!("--option `{name}` is given more than once"))
                    .hint("list every option once"),
            );
        }
    }
    Ok(Value::Object(options))
}

/// The question text, or the structured instructions read from a file.
fn instructions(arguments: &QuestionArgs, context: &mut Context<'_>) -> Result<Value, CliError> {
    match (&arguments.question, &arguments.instructions_file) {
        (_, Some(path)) => structured(path, "--instructions-file", context),
        (Some(text), None) => Ok(json!(text)),
        // `clap` requires one of the two.
        (None, None) => Err(CliError::usage("there is no question")
            .hint("pass the question as text, or --instructions-file <file>")),
    }
}

/// Reads a JSON or YAML file holding structured instructions or criteria.
fn structured(path: &str, flag: &str, context: &mut Context<'_>) -> Result<Value, CliError> {
    if path == "-" {
        return Err(CliError::usage(format!(
            "{flag} cannot read stdin, which is reserved for the state"
        ))
        .hint("give the path of a JSON or YAML file"));
    }
    Ok(input::read_document(path, None, context.stdin)?
        .value()
        .clone())
}

struct Shortcut<'a> {
    question: Value,
    conditions: Vec<Condition>,
    abstain: Option<AbstainBand>,
    hidden_lints: &'a [&'a str],
}

impl Shortcut<'_> {
    fn run(
        self,
        state: &StateArgs,
        send: &SendArgs,
        context: &mut Context<'_>,
    ) -> Result<Exit, CliError> {
        let document = Document::from_value(json!({ "questions": { ANSWER: self.question } }));
        let plan = Plan {
            document,
            source: Source {
                file_has_state: false,
                file_is_stdin: false,
            },
            state,
            send,
            conditions: &self.conditions,
            hidden_lints: self.hidden_lints,
        };
        let Some(evaluation) = evaluate_plan(plan, context)? else {
            return Ok(Exit::Success);
        };

        let outcome = gate::decide(
            &self.conditions,
            self.abstain.map(|band| (ANSWER, band)),
            &evaluation.envelope.answers,
        )?;
        let exit = outcome.as_ref().map_or(Exit::Success, GateOutcome::exit);
        if send.raw {
            context.write_raw(&evaluation.raw_body.unwrap_or_default())?;
        } else {
            let mut envelope = evaluation.envelope;
            envelope.gate = outcome;
            context.output.emit(&Single(envelope), context.stdout)?;
        }
        Ok(exit)
    }
}

/// The result of a shortcut: the one answer's own fields at the top level, so that
/// `--field noul`, `--field choice` and `--field score` work, followed by the same facts about
/// the call that `jev eval` reports.
struct Single(ResultEnvelope);

impl Serialize for Single {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut flat = self
            .0
            .answers
            .get(ANSWER)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Ok(Value::Object(envelope)) = serde_json::to_value(&self.0) {
            flat.extend(envelope.into_iter().filter(|(key, _)| key != "answers"));
        }
        flat.serialize(serializer)
    }
}

impl Render for Single {
    fn human(&self, ui: Ui) -> String {
        self.0.human(ui)
    }
}
