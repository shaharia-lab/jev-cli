//! The tools `jev mcp serve` offers, and their input schemas.
//!
//! Adding a tool means writing its handler and adding it to [`all`]. A tool that should only be
//! offered in some sessions is added there conditionally: a tool that is not listed cannot be
//! called either.

use jev_client::validate::{self, Document, Options};
use jev_client::{Choice, Noul, Request, Score, Transport};
use schemars::JsonSchema;
use serde_json::{Map, Value, json};

use super::roots::Roots;
use super::{Reporter, Session, batch_run, with_session};
use crate::commands::shortcut::{ANSWER, Single};
use crate::commands::validate::Validation;
use crate::error::CliError;
use crate::schemas;

/// What a tool does with its arguments.
pub(super) type Handler<T> =
    fn(&mut Session<T>, &Map<String, Value>, &mut Reporter<'_>) -> Result<Value, CliError>;

/// One tool, as `tools/list` describes it and `tools/call` runs it.
pub(super) struct Tool<T> {
    pub(super) name: &'static str,
    title: &'static str,
    description: String,
    input_schema: Value,
    /// Whether the tool reaches the API (and costs money), rather than working offline.
    open_world: bool,
    /// Whether the tool leaves everything as it was, rather than writing files.
    read_only: bool,
    pub(super) call: Handler<T>,
}

impl<T> Tool<T> {
    /// The tool as `tools/list` shows it.
    pub(super) fn describe(&self) -> Value {
        json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": self.input_schema,
            "annotations": {
                "title": self.title,
                "readOnlyHint": self.read_only,
                "destructiveHint": false,
                "idempotentHint": !self.open_world && self.read_only,
                "openWorldHint": self.open_world
            }
        })
    }
}

/// What every evaluating tool's description ends with: how to get good answers from Jev.
const JEV_LIMITS: &str = "Jev never generates text: it returns calibrated probabilities. It \
cannot count, do arithmetic or compare numbers, and it reads dates as text (ask for the parts, \
such as the month, as choices, and compare in code). It is a literal reader: put the full meaning \
in `instructions`, since question ids are never shown to it. Ask one snap judgment per question; \
split anything that needs reasoning into several questions.";

/// What an evaluating tool returns.
const RESULT: &str = "The result is the same JSON envelope `jev` prints: `model` (the versioned \
model that answered), `requested_model`, `usage`, `cost_usd` (estimated; null when the price is \
unknown), `request_id`, `latency_ms`, `warnings` (validation lints, only when there are any) and \
`session` (calls made and estimated spend so far in this session). The request is validated \
offline first: an invalid one is a tool error listing every finding, and nothing is sent.";

/// Every tool this server offers, in the order `tools/list` shows them. `batch_run` is offered
/// only with directories it may use.
pub(super) fn all<T: Transport + 'static>(batch: Option<&Roots>) -> Vec<Tool<T>> {
    let request = schema_of::<Request>();
    let mut tools = vec![
        evaluate(&request),
        noul(&request),
        choice(&request),
        score(&request),
        validate(&request),
        list_models(),
    ];
    if let Some(roots) = batch {
        tools.push(batch_run(&request, roots));
    }
    tools
}

/// Several questions about one state.
fn evaluate<T: Transport>(request: &Map<String, Value>) -> Tool<T> {
    Tool {
        name: "evaluate",
        title: "Evaluate a state against several questions",
        description: format!(
            "Evaluate one `state` against several typed questions in a single call: `noul` \
             (yes/no), `choice` (one of a set of options) and `score` (a rubric of 2 to 10 \
             levels), keyed by ids you choose. Prefer this over `noul`, `choice` and `score` \
             whenever there is more than one question about the same state, including \
             speculative ones: questions are answered in parallel, and one call is far \
             cheaper and faster than several. {RESULT} `answers` holds one answer per \
             question id, exactly as the API returned it. {JEV_LIMITS} In a choice, always \
             include an escape option such as `other` or `not_stated`."
        ),
        input_schema: evaluate_schema(request),
        open_world: true,
        read_only: true,
        call: evaluate_tool::<T>,
    }
}

/// One yes/no question.
fn noul<T: Transport>(request: &Map<String, Value>) -> Tool<T> {
    Tool {
        name: "noul",
        title: "Ask one yes/no question",
        description: format!(
            "Ask one yes/no question about a `state`. Returns `noul`, the probability that \
             the answer is yes (0 to 1). Choose noul for an absolute judgment, \"does this \
             hold?\", which may be low for every candidate; for tagging, ask one noul per \
             label (in one `evaluate` call). A value near 0.5 means Jev cannot tell, not \
             \"medium\". Use `choice` when exactly one of several options fits, `score` for \
             a position on an ordered scale, and `evaluate` for several questions about the \
             same state. {RESULT} {JEV_LIMITS}"
        ),
        input_schema: single_schema::<Noul>(request),
        open_world: true,
        read_only: true,
        call: noul_tool::<T>,
    }
}

/// One option from a set.
fn choice<T: Transport>(request: &Map<String, Value>) -> Tool<T> {
    Tool {
        name: "choice",
        title: "Pick one option from a set",
        description: format!(
            "Pick the one option that best fits a `state`, from up to 255 options you define \
             in `criteria` (option name to description, or null when the name says enough). \
             Returns `choice` (the winner), `confidence` (how peaked the distribution is, not \
             how correct it is) and `probabilities` per option. Choose choice for a relative \
             judgment, \"which one of these?\". Always include an escape option such as \
             `other`, `none_of_the_above` or `not_stated`: Jev cannot pick an option you \
             left out. Use `noul` per label when several can apply at once, `score` for an \
             ordered scale, and `evaluate` for several questions about the same state. \
             {RESULT} {JEV_LIMITS}"
        ),
        input_schema: single_schema::<Choice>(request),
        open_world: true,
        read_only: true,
        call: choice_tool::<T>,
    }
}

/// A position on an ordered rubric.
fn score<T: Transport>(request: &Map<String, Value>) -> Tool<T> {
    Tool {
        name: "score",
        title: "Rate on an ordered rubric",
        description: format!(
            "Rate a `state` on an ordered rubric of 2 to 10 levels, lowest first, given in \
             `criteria`. Returns `score` (the expected level, from 0 to the number of levels \
             minus 1), `confidence`, `probabilities` and `legend`, both keyed by the level \
             number as a string. Each level is judged on its own, so describe a situation \
             for each (\"answers the question with a working example\"), not a degree \
             (\"better than the previous\"); rate one dimension per score, and do not use it \
             to extract a number. Use `choice` for unordered options, `noul` for yes/no, and \
             `evaluate` for several questions about the same state. {RESULT} {JEV_LIMITS}"
        ),
        input_schema: single_schema::<Score>(request),
        open_world: true,
        read_only: true,
        call: score_tool::<T>,
    }
}

/// Offline checking.
fn validate<T: Transport>(request: &Map<String, Value>) -> Tool<T> {
    Tool {
        name: "validate",
        title: "Check a request offline",
        description: "Check a request offline, with every rule `evaluate` applies plus \
            lints for common mistakes (a choice without an escape option, a misspelt field). \
            Nothing is sent, nothing is billed, and no API key is needed. Takes the same \
            arguments as `evaluate`, with `state` optional (give it to get a size estimate). \
            Returns `valid` (false only when there are errors; warnings do not count), \
            `summary` with the counts, `findings` (each with `severity`, `rule`, `question`, \
            a JSON Pointer `path`, `message` and `suggestion`) and `size`, the estimated \
            input tokens. Use it while writing or fixing a request; `evaluate`, `noul`, \
            `choice` and `score` run the same checks before every call anyway."
            .to_owned(),
        input_schema: validate_schema(request),
        open_world: false,
        read_only: true,
        call: validate_tool::<T>,
    }
}

/// The model names.
fn list_models<T: Transport>() -> Tool<T> {
    Tool {
        name: "list_models",
        title: "List the available models",
        description: "List the model names and aliases the account can use, with a \
            description and release date: `{\"models\": [{\"name\", \"description\", \
            \"release_date\"}]}`. Use it to pick a `model` for the other tools. Aliases such \
            as `jev-latest` move to newer versions without notice; a versioned id such as \
            `jev-1.13.0` is accepted even when it is not listed, and every evaluation \
            reports the versioned `model` that answered."
            .to_owned(),
        input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        open_world: true,
        read_only: true,
        call: list_models_tool::<T>,
    }
}

/// One question set over every row of a file.
fn batch_run<T: Transport + 'static>(request: &Map<String, Value>, roots: &Roots) -> Tool<T> {
    Tool {
        name: "batch_run",
        title: "Evaluate every row of a file",
        description: format!(
            "Apply one set of questions to every row of a JSONL, CSV or JSON array file, one \
             API call per row, and write one JSON record per row to a new file, exactly as `jev batch run \
             --out` does. Use it for many states (tens to thousands); use `evaluate` for one \
             state, and `validate` to check the questions first. Files are read and written \
             only inside {}; a relative path is taken from the first. `out` must not exist \
             yet. Each row's state is the whole row, one field (`state_field`) or an object of \
             some fields (`state_fields`). Every row is checked and priced before anything is \
             sent: a bad row, or a run over the server's row or cost limit, is a tool error and \
             costs nothing. Progress is reported when the call has a `progressToken`. The \
             result is the run's summary: `rows_total`, `ok`, `failed` (rows whose record is \
             an error), `skipped`, `input_tokens`, `cost_usd` (estimated), `wall_time_ms`, \
             `rows_per_second`, `retries`, `models`, `stopped_by`, then `out` (the records \
             file), `warnings` and `session`. Each record has `id`, `status` (`ok` or \
             `error`) and either the answers or an `error`. {JEV_LIMITS}",
            roots.listed()
        ),
        input_schema: batch_run_schema(request),
        open_world: true,
        read_only: false,
        call: batch_run::call::<T>,
    }
}

fn evaluate_tool<T: Transport>(
    session: &mut Session<T>,
    arguments: &Map<String, Value>,
    _: &mut Reporter<'_>,
) -> Result<Value, CliError> {
    // The arguments are the request itself: validation reports anything that does not belong.
    let document = Document::from_value(Value::Object(arguments.clone()));
    let evaluated = session.evaluate(document)?;
    with_session(
        &evaluated.evaluation.envelope,
        &evaluated.warnings,
        session.spend(),
    )
}

fn noul_tool<T: Transport>(
    session: &mut Session<T>,
    arguments: &Map<String, Value>,
    _: &mut Reporter<'_>,
) -> Result<Value, CliError> {
    single(session, "noul", arguments)
}

fn choice_tool<T: Transport>(
    session: &mut Session<T>,
    arguments: &Map<String, Value>,
    _: &mut Reporter<'_>,
) -> Result<Value, CliError> {
    single(session, "choice", arguments)
}

fn score_tool<T: Transport>(
    session: &mut Session<T>,
    arguments: &Map<String, Value>,
    _: &mut Reporter<'_>,
) -> Result<Value, CliError> {
    single(session, "score", arguments)
}

/// The arguments a one-question tool takes: the request's, and the question's.
const SINGLE_ARGUMENTS: [&str; 4] = ["state", "instructions", "criteria", "model"];

/// Runs one question, the way `jev noul`, `jev choice` and `jev score` do: as a one-question
/// request whose answer is flattened into the result.
fn single<T: Transport>(
    session: &mut Session<T>,
    kind: &str,
    arguments: &Map<String, Value>,
) -> Result<Value, CliError> {
    if let Some(unknown) = arguments
        .keys()
        .find(|key| !SINGLE_ARGUMENTS.contains(&key.as_str()))
    {
        return Err(
            CliError::usage(format!("`{kind}` has no argument `{unknown}`")).hint(format!(
                "the arguments are {}",
                SINGLE_ARGUMENTS.map(|name| format!("`{name}`")).join(", ")
            )),
        );
    }
    let mut question = Map::new();
    question.insert("type".to_owned(), json!(kind));
    let mut request = Map::new();
    for (name, value) in arguments {
        match name.as_str() {
            "instructions" | "criteria" => question.insert(name.clone(), value.clone()),
            _ => request.insert(name.clone(), value.clone()),
        };
    }
    request.insert("questions".to_owned(), json!({ ANSWER: question }));

    let evaluated = session.evaluate(Document::from_value(Value::Object(request)))?;
    with_session(
        &Single(evaluated.evaluation.envelope),
        &evaluated.warnings,
        session.spend(),
    )
}

fn validate_tool<T: Transport>(
    _: &mut Session<T>,
    arguments: &Map<String, Value>,
    _: &mut Reporter<'_>,
) -> Result<Value, CliError> {
    let document = Document::from_value(Value::Object(arguments.clone()));
    // As `jev validate` does: the model and the state may be left to whoever evaluates it.
    let options = Options::default().model_optional(true).state_optional(true);
    let report = validate::check_document(&document, &options);
    serde_json::to_value(Validation::new("request", report, false))
        .map_err(|error| CliError::internal(format!("could not encode a result: {error}")))
}

fn list_models_tool<T: Transport>(
    session: &mut Session<T>,
    _: &Map<String, Value>,
    _: &mut Reporter<'_>,
) -> Result<Value, CliError> {
    let transport = session.transport.as_ref().map_err(Clone::clone)?;
    let reply = session.runtime.block_on(transport.list_models())?;
    Ok(json!({ "models": reply.body.models }))
}

/// A type's JSON Schema as a plain object, with every definition inlined: some MCP clients do
/// not follow `$ref`.
fn schema_of<T: JsonSchema>() -> Map<String, Value> {
    let mut schema = schemas::inlined::<T>();
    schema.remove("$schema");
    schema.remove("title");
    schema
}

/// What a tool says about `model`, which the request type describes for a request file.
const MODEL: &str = "The model: an alias such as `jev-latest` or a versioned id such as \
`jev-1.13.0`. Optional: the server's configured model is used when it is left out, and a server \
started with --model always uses that one. Aliases move without notice; the result reports the \
versioned `model` that answered.";

/// The schema of the request, with `model` optional.
fn evaluate_schema(request: &Map<String, Value>) -> Value {
    let mut schema = request.clone();
    if let Some(Value::Object(model)) = schema
        .get_mut("properties")
        .and_then(|properties| properties.get_mut("model"))
    {
        model.insert("description".to_owned(), json!(MODEL));
    }
    schema.insert("required".to_owned(), json!(["state", "questions"]));
    Value::Object(schema)
}

/// The schema of `batch_run`: its own arguments, with `questions` and `model` as a request has
/// them.
fn batch_run_schema(request: &Map<String, Value>) -> Value {
    let mut schema = schema_of::<batch_run::Arguments>();
    schema.remove("description");
    if let Some(Value::Object(properties)) = schema.get_mut("properties") {
        for name in ["questions", "model"] {
            let own = properties
                .get(name)
                .and_then(|own| own.get("description"))
                .cloned();
            if let Some(Value::Object(from_request)) = request
                .get("properties")
                .and_then(|properties| properties.get(name))
            {
                let mut property = from_request.clone();
                let description = if name == "model" {
                    Some(json!(MODEL))
                } else {
                    own
                };
                if let Some(description) = description {
                    property.insert("description".to_owned(), description);
                }
                properties.insert(name.to_owned(), Value::Object(property));
            }
        }
    }
    Value::Object(schema)
}

/// The schema of `validate`: a request whose state and model may be left out.
fn validate_schema(request: &Map<String, Value>) -> Value {
    let mut schema = match evaluate_schema(request) {
        Value::Object(schema) => schema,
        _ => Map::new(),
    };
    schema.insert("required".to_owned(), json!(["questions"]));
    Value::Object(schema)
}

/// The schema of a one-question tool: the request's `state`, the question's own fields, and the
/// request's `model`, flat.
fn single_schema<Q: JsonSchema>(request: &Map<String, Value>) -> Value {
    let question = schema_of::<Q>();
    let request_property = |name: &str| {
        request
            .get("properties")
            .and_then(|properties| properties.get(name))
            .cloned()
            .unwrap_or_else(|| json!({}))
    };
    let mut properties = Map::new();
    properties.insert("state".to_owned(), request_property("state"));
    if let Some(Value::Object(own)) = question.get("properties") {
        properties.extend(own.clone());
    }
    let mut model = request_property("model");
    if let Value::Object(model) = &mut model {
        model.insert("description".to_owned(), json!(MODEL));
    }
    properties.insert("model".to_owned(), model);

    let mut required = vec![json!("state")];
    if let Some(Value::Array(own)) = question.get("required") {
        required.extend(own.iter().cloned());
    }
    let mut schema = Map::new();
    schema.insert("type".to_owned(), json!("object"));
    if let Some(description) = question.get("description") {
        schema.insert("description".to_owned(), description.clone());
    }
    schema.insert("properties".to_owned(), Value::Object(properties));
    schema.insert("required".to_owned(), Value::Array(required));
    schema.insert("additionalProperties".to_owned(), json!(false));
    Value::Object(schema)
}

#[cfg(test)]
mod tests {
    use jev_client::HttpTransport;
    use serde_json::{Value, json};

    use super::all;

    fn tools() -> Vec<Value> {
        all::<HttpTransport>(None)
            .iter()
            .map(super::Tool::describe)
            .collect()
    }

    fn tool(name: &str) -> Value {
        tools()
            .into_iter()
            .find(|tool| tool["name"] == name)
            .unwrap()
    }

    #[test]
    fn the_tools_are_the_ones_the_prd_names() {
        let names: Vec<Value> = tools().iter().map(|tool| tool["name"].clone()).collect();

        assert_eq!(
            names,
            [
                "evaluate",
                "noul",
                "choice",
                "score",
                "validate",
                "list_models"
            ]
        );
    }

    #[test]
    fn every_schema_is_a_self_contained_object_schema() {
        for tool in tools() {
            let schema = &tool["inputSchema"];
            assert_eq!(schema["type"], "object", "{}", tool["name"]);
            assert!(
                !schema.to_string().contains("$ref"),
                "{} refers to a definition: {schema}",
                tool["name"]
            );
        }
    }

    #[test]
    fn a_one_question_schema_is_the_question_plus_the_state_and_model_of_a_request() {
        let schema = &tool("score")["inputSchema"];

        let properties: Vec<&str> = schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(properties, ["state", "instructions", "criteria", "model"]);
        assert_eq!(
            schema["required"],
            json!(["state", "instructions", "criteria"])
        );
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["properties"]["criteria"]["items"]["type"],
            json!(["string", "object", "array"]),
            "a level cannot be null, as in the request type"
        );
        assert_eq!(
            tool("noul")["inputSchema"]["required"],
            json!(["state", "instructions"])
        );
    }

    #[test]
    fn evaluate_needs_a_state_and_questions_and_validate_only_questions() {
        assert_eq!(
            tool("evaluate")["inputSchema"]["required"],
            json!(["state", "questions"])
        );
        assert_eq!(
            tool("validate")["inputSchema"]["required"],
            json!(["questions"])
        );
        assert_eq!(
            tool("evaluate")["inputSchema"]["additionalProperties"],
            false
        );
    }

    #[test]
    fn every_description_says_when_to_choose_it_and_what_jev_cannot_do() {
        for name in ["evaluate", "noul", "choice", "score"] {
            let description = tool(name)["description"].as_str().unwrap().to_owned();
            assert!(description.contains("arithmetic"), "{name}");
            assert!(description.contains("dates"), "{name}");
            assert!(
                description.contains("escape option") || name == "noul" || name == "score",
                "{name}"
            );
            for sibling in ["evaluate", "noul", "choice", "score"] {
                if sibling != name {
                    assert!(
                        description.contains(&format!("`{sibling}`")),
                        "{name} does not say when to use {sibling}"
                    );
                }
            }
        }
    }
}
