//! `jev eval`.

use std::fmt::Write as _;

use jev_client::validate::{Finding, SizeEstimate};
use jev_client::{Request, Transport};
use serde::Serialize;

use super::Context;
use crate::cli::EvalArgs;
use crate::config::{Key, Value as SettingValue};
use crate::error::CliError;
use crate::evaluate::{self, PrepareOptions, Prepared};
use crate::input::{self, StateSource};
use crate::output::{Render, Ui};

pub(crate) fn run(arguments: &EvalArgs, context: &mut Context<'_>) -> Result<(), CliError> {
    let settings = context.settings()?.clone();
    let document = input::read_document(&arguments.file, arguments.input_format, context.stdin)?;

    let file_is_stdin = arguments.file == "-";
    let state_source = state_source(
        arguments,
        document.value().get("state").is_some(),
        file_is_stdin,
        context.interaction.stdin_is_terminal(),
    )?;
    let state = state_source
        .map(|source| input::read_state(&source, arguments.state_format, context.stdin))
        .transpose()?;

    let options = PrepareOptions {
        state,
        strict: arguments.strict,
        skip_size_check: arguments.skip_size_check,
    };
    let prepared = evaluate::prepare(document, &settings, &options)?;
    for warning in prepared.warnings() {
        context.notify(&warning);
    }

    if arguments.dry_run {
        let base_url = settings
            .get(Key::BaseUrl)
            .value
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        let preview = Preview::new(&prepared, &base_url);
        return context.output.emit(&preview, context.stdout);
    }

    let transport = context.transport(&settings)?;
    let price = context.config_file()?.pricing_usd_per_mtok;
    let evaluation = block_on(evaluate::send(&transport, &prepared, price))?;

    let remind = arguments.warn_unpinned
        || settings.get(Key::WarnUnpinned).value == Some(SettingValue::Switch(true));
    if let (true, Some(notice)) = (remind, evaluation.unpinned_notice()) {
        context.notify(&notice);
    }
    if arguments.raw {
        let raw = evaluation.raw_body.unwrap_or_default();
        return context.write_raw(&raw);
    }
    context.output.emit(&evaluation.envelope, context.stdout)
}

/// Decides where the state comes from, when it is not the request file.
///
/// `--state` and `--state-file` win over the file. A pipe is read only when nothing else supplies
/// a state: reading stdin whenever it is not a terminal would hang every caller that leaves it
/// open, which agents and CI commonly do. `--state-file -` is the explicit way to pipe one in.
fn state_source(
    arguments: &EvalArgs,
    file_has_state: bool,
    file_is_stdin: bool,
    stdin_is_terminal: bool,
) -> Result<Option<StateSource>, CliError> {
    let stdin_taken = || {
        CliError::usage("stdin is already being read as the request file (-f -), so it cannot also supply the state")
            .hint("pass the state with --state or --state-file <path>, or put it in the request")
    };
    if let Some(text) = &arguments.state {
        return Ok(Some(StateSource::Inline(text.clone())));
    }
    if let Some(path) = &arguments.state_file {
        return match (path.as_str(), file_is_stdin) {
            ("-", true) => Err(stdin_taken()),
            ("-", false) => Ok(Some(StateSource::Stdin)),
            _ => Ok(Some(StateSource::File(path.clone()))),
        };
    }
    if file_has_state {
        return Ok(None);
    }
    if file_is_stdin {
        return Err(stdin_taken());
    }
    if stdin_is_terminal {
        return Err(CliError::usage("there is no state to evaluate")
            .hint("pass --state <text> or --state-file <path>, pipe the state in, or add `state` to the request file"));
    }
    Ok(Some(StateSource::Stdin))
}

/// Runs one future to completion on a runtime of its own.
fn block_on<T>(future: impl Future<Output = Result<T, CliError>>) -> Result<T, CliError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| CliError::internal(format!("could not start the async runtime: {error}")))?
        .block_on(future)
}

/// Lists the models, through whatever transport the caller has.
pub(crate) fn list_models(transport: &impl Transport) -> Result<jev_client::ModelList, CliError> {
    block_on(async { Ok(transport.list_models().await?.body) })
}

/// What `--dry-run` prints: everything that would be sent, and nothing that was.
#[derive(Debug, Serialize)]
struct Preview {
    /// Always `true`: nothing was sent, and nothing was billed.
    dry_run: bool,
    /// Where the request would go.
    endpoint: String,
    /// The model name in the request, and what chose it.
    requested_model: String,
    model_origin: String,
    /// The exact request body. The API key is not part of it, or of anything printed here.
    body: Request,
    /// An estimate of the input tokens, unless the size check was skipped.
    size_estimate: Option<SizeEstimate>,
    /// Validation warnings. There are no errors, or this would not have been reached.
    warnings: Vec<Finding>,
}

impl Preview {
    fn new(prepared: &Prepared, base_url: &str) -> Self {
        Self {
            dry_run: true,
            endpoint: format!("{}/v1/systemone", base_url.trim_end_matches('/')),
            requested_model: prepared.requested_model.clone(),
            model_origin: prepared.model_origin.clone(),
            body: prepared.request.clone(),
            size_estimate: prepared.report.size.clone(),
            warnings: prepared.report.warnings().cloned().collect(),
        }
    }
}

impl Render for Preview {
    fn human(&self, ui: Ui) -> String {
        let body = serde_json::to_string_pretty(&self.body).unwrap_or_default();
        let mut text = format!(
            "{}\nPOST {}\nmodel {} (from {})\n",
            ui.bold("dry run: nothing was sent"),
            self.endpoint,
            self.requested_model,
            self.model_origin
        );
        if let Some(size) = &self.size_estimate {
            let _ = writeln!(
                text,
                "estimated input tokens: {} of {} in total, {} of {} for the state plus the longest question",
                size.total_tokens,
                size.total_budget_tokens,
                size.largest_question_tokens,
                size.question_budget_tokens
            );
        }
        format!("{text}{body}\n")
    }
}
