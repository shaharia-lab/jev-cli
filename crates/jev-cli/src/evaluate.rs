//! The evaluation service: from a request document to a result, the same way for every caller.
//!
//! `jev eval` uses it directly. The shortcut commands, batch runs and the MCP server build their
//! own request documents and then go through the same three steps, so that model resolution,
//! validation and the result envelope cannot drift apart between them:
//!
//! 1. [`prepare`]: fill in the model and the state, validate offline, produce a typed request.
//! 2. [`send`]: one API call through a [`Transport`].
//! 3. The [`ResultEnvelope`] every output format renders.

use jev_client::validate::{self, Document, Finding, Options, Report, Severity};
use jev_client::{Request, Transport};
use serde_json::{Value, json};

use crate::config::{Key, Settings, Source};
use crate::error::CliError;
use crate::notice::Notice;
use crate::output::ResultEnvelope;

/// How many findings an error spells out for a person before counting the rest.
const MAX_LISTED_FINDINGS: usize = 8;

/// A request that has passed validation and is ready to send.
#[derive(Debug)]
pub(crate) struct Prepared {
    pub(crate) request: Request,
    /// The model name the request carries, which may be an alias.
    pub(crate) requested_model: String,
    /// Where that model name came from: `--model`, the request file, a variable, the profile.
    pub(crate) model_origin: String,
    /// The validation report. It has no errors, but may have warnings and a size estimate.
    pub(crate) report: Report,
}

impl Prepared {
    /// The validation warnings, as notices for stderr.
    pub(crate) fn warnings(&self) -> Vec<Notice> {
        self.report
            .warnings()
            .map(|finding| {
                let place = finding
                    .question
                    .as_ref()
                    .map_or_else(String::new, |id| format!("question `{id}`: "));
                let notice = Notice::warning(
                    "request_lint",
                    format!("{place}{} [{}]", finding.message, finding.rule.id()),
                );
                match &finding.suggestion {
                    Some(suggestion) => notice.hint(suggestion.clone()),
                    None => notice,
                }
            })
            .collect()
    }
}

/// How a request is prepared.
#[derive(Clone, Debug, Default)]
pub(crate) struct PrepareOptions {
    /// The state to use instead of the document's, already in its final form.
    pub(crate) state: Option<Value>,
    /// Treat validation warnings as errors.
    pub(crate) strict: bool,
    /// Skip the size estimate.
    pub(crate) skip_size_check: bool,
}

/// Fills in the model and the state, validates, and produces the typed request.
///
/// The model is resolved as **`--model`, then the document's `model`, then
/// `TYPESAFE_DEFAULT_MODEL`, then the profile, then `jev-latest`**: a file that names a model is
/// more specific than an ambient default, so it outranks the environment and the profile.
///
/// # Errors
///
/// A usage error (exit 2) listing what is wrong with the request. Nothing has been sent.
pub(crate) fn prepare(
    mut document: Document,
    settings: &Settings,
    options: &PrepareOptions,
) -> Result<Prepared, CliError> {
    let chosen = settings.get(Key::Model);
    let from_settings = chosen
        .value
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    let in_document = document
        .value()
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let (requested_model, model_origin) = match (&chosen.source, in_document) {
        (Source::Flag(flag), _) => (from_settings, (*flag).to_owned()),
        (_, Some(model)) => (model, "the request file".to_owned()),
        (source, None) => (
            from_settings,
            source
                .origin()
                .unwrap_or_else(|| "the built-in default".to_owned()),
        ),
    };

    if let Value::Object(root) = document.value_mut() {
        // A document that already has a `model` of the wrong type is left for validation to report.
        if root.get("model").is_none_or(Value::is_string) {
            root.insert("model".to_owned(), json!(requested_model));
        }
        if let Some(state) = &options.state {
            root.insert("state".to_owned(), state.clone());
        }
    }

    let validation = Options::default()
        .strict(options.strict)
        .skip_size_check(options.skip_size_check);
    let report = validate::check_document(&document, &validation);
    if !report.is_valid() {
        return Err(invalid_request(&report));
    }
    let request = document.into_request().map_err(|error| {
        // Validation accepted it, so this is a disagreement between validation and the types.
        CliError::internal(format!(
            "a request that passed validation could not be built: {error}"
        ))
    })?;
    Ok(Prepared {
        request,
        requested_model,
        model_origin,
        report,
    })
}

/// The error for a request that failed validation.
fn invalid_request(report: &Report) -> CliError {
    let errors: Vec<&Finding> = report.errors().collect();
    let count = errors.len();
    let mut error = CliError::usage(format!(
        "the request is not valid: {count} problem{} found, nothing was sent",
        if count == 1 { "" } else { "s" }
    ))
    .hint("fix the request; `jev validate -f <file>` gives the full report, warnings included");
    for finding in errors.iter().take(MAX_LISTED_FINDINGS) {
        let place = if finding.path.is_empty() {
            "request"
        } else {
            finding.path.as_str()
        };
        error.lines.push(format!(
            "- {place}: {} [{}]",
            finding.message,
            finding.rule.id()
        ));
        if let Some(suggestion) = &finding.suggestion {
            error.lines.push(format!("  fix: {suggestion}"));
        }
    }
    if count > MAX_LISTED_FINDINGS {
        error
            .lines
            .push(format!("- and {} more", count - MAX_LISTED_FINDINGS));
    }
    let listed: Vec<&Finding> = report
        .findings
        .iter()
        .filter(|finding| finding.severity == Severity::Error)
        .collect();
    error.details = serde_json::to_value(json!({ "findings": listed })).ok();
    error
}

/// Sends a prepared request and wraps the answer.
///
/// # Errors
///
/// Whatever the transport fails with, mapped to the exit-code contract.
pub(crate) async fn send(
    transport: &impl Transport,
    prepared: &Prepared,
    usd_per_mtok: Option<f64>,
) -> Result<Evaluation, CliError> {
    let reply = transport.evaluate(&prepared.request).await?;
    let raw_value = reply
        .raw_body
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
    let envelope = ResultEnvelope::new(
        &reply.body,
        raw_value.as_ref(),
        &prepared.requested_model,
        reply.meta.request_id.clone(),
        reply.meta.latency,
        usd_per_mtok,
    );
    Ok(Evaluation {
        envelope,
        raw_body: reply.raw_body,
        attempts: reply.meta.attempts,
    })
}

/// The outcome of one evaluation.
#[derive(Debug)]
pub(crate) struct Evaluation {
    pub(crate) envelope: ResultEnvelope,
    /// The response body exactly as received, for `--raw`.
    pub(crate) raw_body: Option<String>,
    /// How many API calls it took, retries included.
    pub(crate) attempts: u32,
}

impl Evaluation {
    /// A reminder to pin the model, when the request used an alias.
    pub(crate) fn unpinned_notice(&self) -> Option<Notice> {
        unpinned_notice(&self.envelope.requested_model, &self.envelope.model)
    }
}

/// A reminder to pin the model, when `requested` is an alias that `resolved` answered for.
pub(crate) fn unpinned_notice(requested: &str, resolved: &str) -> Option<Notice> {
    (requested != resolved).then(|| {
        Notice::warning(
            "unpinned_model",
            format!("`{requested}` is an alias, answered this time by `{resolved}`; aliases move without notice, and thresholds tuned on one version may not hold on the next"),
        )
        .hint(format!("pin the version: --model {resolved}, or `jev config set model {resolved}`"))
    })
}

#[cfg(test)]
mod tests {
    use std::future::{Future, ready};
    use std::time::Duration;

    use indexmap::IndexMap;
    use jev_client::validate::Document;
    use jev_client::{
        Answer, Error, ErrorKind, ModelList, NoulAnswer, Reply, ReplyMeta, Request, Response,
        Transport, Usage,
    };
    use serde_json::json;

    use super::{PrepareOptions, prepare, send};
    use crate::config::{Flags, Settings};
    use crate::env::Env;

    fn settings(flags: &Flags, env: &Env) -> Settings {
        Settings::resolve(flags, env, None, &IndexMap::new()).unwrap()
    }

    fn document(json: &serde_json::Value) -> Document {
        Document::from_value(json.clone())
    }

    fn noul() -> serde_json::Value {
        json!({ "q": { "type": "noul", "instructions": "Is it urgent?" } })
    }

    #[test]
    fn the_model_is_flag_then_file_then_environment_then_default() {
        let with_model = json!({ "state": "s", "model": "file-model", "questions": noul() });
        let without = json!({ "state": "s", "questions": noul() });
        let flag = Flags {
            model: Some("flag-model".into()),
            ..Flags::default()
        };
        let env = Env::from([("TYPESAFE_DEFAULT_MODEL", "env-model")]);
        let options = PrepareOptions::default();

        let cases = [
            (&with_model, &flag, &env, "flag-model", "--model"),
            (
                &with_model,
                &Flags::default(),
                &env,
                "file-model",
                "the request file",
            ),
            (
                &without,
                &Flags::default(),
                &env,
                "env-model",
                "TYPESAFE_DEFAULT_MODEL",
            ),
            (
                &without,
                &Flags::default(),
                &Env::default(),
                "jev-latest",
                "the built-in default",
            ),
        ];
        for (json, flags, env, model, origin) in cases {
            let prepared = prepare(document(json), &settings(flags, env), &options).unwrap();

            assert_eq!(
                (
                    prepared.requested_model.as_str(),
                    prepared.model_origin.as_str()
                ),
                (model, origin)
            );
            assert_eq!(
                prepared.request.model, model,
                "the request carries the resolved model"
            );
        }
    }

    #[test]
    fn a_state_given_outside_the_file_replaces_the_files() {
        let json = json!({ "state": "from the file", "questions": noul() });
        let options = PrepareOptions {
            state: Some(json!({ "from": "a flag" })),
            ..PrepareOptions::default()
        };

        let prepared = prepare(
            document(&json),
            &settings(&Flags::default(), &Env::default()),
            &options,
        )
        .unwrap();

        assert_eq!(
            prepared.request.state.as_value(),
            &json!({ "from": "a flag" })
        );
    }

    #[test]
    fn an_invalid_request_is_a_usage_error_that_lists_every_problem() {
        let levels: Vec<String> = (0..11).map(|level| format!("Level {level}")).collect();
        let json = json!({ "questions": { "rating": { "type": "score", "instructions": "?", "criteria": levels } } });

        let error = prepare(
            document(&json),
            &settings(&Flags::default(), &Env::default()),
            &PrepareOptions::default(),
        )
        .unwrap_err();

        assert_eq!(error.exit.code(), 2);
        assert_eq!(
            error.message,
            "the request is not valid: 2 problems found, nothing was sent"
        );
        assert!(
            error.lines[0].starts_with("- request: `state` is missing [state-missing]"),
            "{:?}",
            error.lines
        );
        assert!(
            error
                .lines
                .iter()
                .any(|line| line.contains("[score-too-many-levels]")),
            "{:?}",
            error.lines
        );
        let findings = &error.details.unwrap()["findings"];
        assert_eq!(findings[1]["rule"], "score-too-many-levels");
        assert_eq!(findings[1]["question"], "rating");
    }

    #[test]
    fn strict_mode_turns_a_lint_into_a_refusal_and_warnings_are_otherwise_notices() {
        let json = json!({ "state": "s", "questions": { "team": {
            "type": "choice", "instructions": "Which?", "criteria": { "a": "x", "b": "y", "c": "z" }
        }}});
        let current = settings(&Flags::default(), &Env::default());

        let relaxed = prepare(document(&json), &current, &PrepareOptions::default()).unwrap();
        let strict = prepare(
            document(&json),
            &current,
            &PrepareOptions {
                strict: true,
                ..PrepareOptions::default()
            },
        );

        let warnings = relaxed.warnings();
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].message.starts_with("question `team`: ")
                && warnings[0].message.ends_with("[choice-no-escape-option]")
        );
        assert!(warnings[0].hint.as_deref().unwrap().contains("other"));
        assert_eq!(strict.unwrap_err().exit.code(), 2);
    }

    /// A transport that answers from memory.
    struct Canned(Result<&'static str, ErrorKind>);

    impl Transport for Canned {
        fn evaluate(
            &self,
            request: &Request,
        ) -> impl Future<Output = Result<Reply<Response>, Error>> + Send {
            ready(match self.0 {
                Ok(model) => {
                    let answers =
                        IndexMap::from([("q".to_owned(), Answer::from(NoulAnswer::new(0.9)))]);
                    let body = Response::new(model, answers, Usage::new(1_000_000, 10));
                    let raw = json!({ "model": model, "answers": { "q": { "type": "noul", "noul": 0.9, "new_field": 1 } } });
                    let _ = request;
                    Ok(Reply::new(
                        body,
                        ReplyMeta::new(Some("req_1".into()), Duration::from_millis(120), 1),
                    )
                    .with_raw_body(raw.to_string()))
                }
                Err(kind) => Err(Error::new(kind, "nope").with_status(429).with_attempts(3)),
            })
        }

        fn list_models(&self) -> impl Future<Output = Result<Reply<ModelList>, Error>> + Send {
            ready(Ok(Reply::new(ModelList::default(), ReplyMeta::default())))
        }
    }

    fn block_on<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn a_result_carries_the_resolved_model_cost_request_id_and_untouched_answers() {
        let json = json!({ "state": "s", "questions": noul() });
        let prepared = prepare(
            document(&json),
            &settings(&Flags::default(), &Env::default()),
            &PrepareOptions::default(),
        )
        .unwrap();

        let evaluation = block_on(send(&Canned(Ok("jev-1.13.0")), &prepared, None)).unwrap();

        let envelope = serde_json::to_value(&evaluation.envelope).unwrap();
        assert_eq!(envelope["model"], "jev-1.13.0");
        assert_eq!(envelope["requested_model"], "jev-latest");
        assert_eq!(envelope["cost_usd"], 0.042);
        assert_eq!(envelope["request_id"], "req_1");
        assert_eq!(envelope["latency_ms"], 120);
        assert_eq!(
            envelope["answers"]["q"]["new_field"], 1,
            "answers pass through unmodified"
        );
        assert!(evaluation.raw_body.is_some());

        let notice = evaluation.unpinned_notice().unwrap();
        assert!(
            notice
                .message
                .starts_with("`jev-latest` is an alias, answered this time by `jev-1.13.0`")
        );
        assert!(notice.hint.unwrap().contains("--model jev-1.13.0"));
    }

    #[test]
    fn a_pinned_model_gets_no_reminder_and_a_price_override_is_used() {
        let json = json!({ "state": "s", "model": "jev-9.9.9", "questions": noul() });
        let prepared = prepare(
            document(&json),
            &settings(&Flags::default(), &Env::default()),
            &PrepareOptions::default(),
        )
        .unwrap();

        let unknown = block_on(send(&Canned(Ok("jev-9.9.9")), &prepared, None)).unwrap();
        let overridden = block_on(send(&Canned(Ok("jev-9.9.9")), &prepared, Some(0.03))).unwrap();

        assert!(unknown.unpinned_notice().is_none());
        assert_eq!(
            unknown.envelope.cost_usd, None,
            "an unknown model has no price, never a guess"
        );
        assert_eq!(overridden.envelope.cost_usd, Some(0.03));
    }

    #[test]
    fn a_transport_error_keeps_its_exit_code() {
        let json = json!({ "state": "s", "questions": noul() });
        let prepared = prepare(
            document(&json),
            &settings(&Flags::default(), &Env::default()),
            &PrepareOptions::default(),
        )
        .unwrap();

        let error =
            block_on(send(&Canned(Err(ErrorKind::RateLimit)), &prepared, None)).unwrap_err();

        assert_eq!(error.exit.code(), 5);
        assert!(
            error.message.ends_with("(after 3 attempts)"),
            "{}",
            error.message
        );
    }
}
