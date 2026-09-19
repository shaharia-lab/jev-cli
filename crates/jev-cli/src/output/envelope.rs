//! The result of an evaluation, as every output format presents it.

// Until `jev eval` exists (issue #9) only the test hooks build an envelope, so a build without
// them sees this module as unused.
#![cfg_attr(not(feature = "internal-test-hooks"), allow(dead_code))]

use std::fmt::Write as _;
use std::time::Duration;

use indexmap::IndexMap;
use jev_client::{Answer, Response, Usage, pricing};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

use super::table::{Cell, Table};
use super::{Render, Ui};

/// Cells in a probability bar.
const BAR_WIDTH: usize = 20;

/// The documented result of an evaluation.
///
/// This shape is versioned public API. `answers` is what the API returned, passed through
/// unmodified, so a field this version of `jev` does not know still reaches the caller.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub(crate) struct ResultEnvelope {
    /// The versioned id of the model that answered, such as `jev-1.13.0`.
    pub(crate) model: String,
    /// The model name the request used, which may be an alias such as `jev-latest`.
    pub(crate) requested_model: String,
    /// One answer per question, exactly as the API returned them.
    pub(crate) answers: IndexMap<String, Value>,
    /// Token usage. Only input tokens are billed.
    pub(crate) usage: Usage,
    /// The estimated cost in US dollars, or `null` when the model's price is not known.
    pub(crate) cost_usd: Option<f64>,
    /// The `x-typesafe-request-id` to quote when asking TypeSafe about this request.
    pub(crate) request_id: Option<String>,
    /// Wall-clock time for the call, including retries, in milliseconds.
    pub(crate) latency_ms: u64,
}

impl ResultEnvelope {
    /// Builds the envelope from a response body as received and the facts about the exchange.
    ///
    /// `raw` is the response body; its `answers` are passed through. When it has none (a fake
    /// transport, say) the typed answers are serialised instead.
    pub(crate) fn new(
        response: &Response,
        raw: Option<&Value>,
        requested_model: &str,
        request_id: Option<String>,
        latency: Duration,
    ) -> Self {
        let passed_through = raw
            .and_then(|raw| raw.get("answers"))
            .and_then(Value::as_object);
        let answers = match passed_through {
            Some(answers) => answers
                .iter()
                .map(|(id, answer)| (id.clone(), answer.clone()))
                .collect(),
            None => response
                .answers
                .iter()
                .map(|(id, answer)| {
                    (
                        id.clone(),
                        serde_json::to_value(answer).unwrap_or(Value::Null),
                    )
                })
                .collect(),
        };
        Self {
            model: response.model.clone(),
            requested_model: requested_model.to_owned(),
            answers,
            usage: response.usage,
            cost_usd: pricing::estimate_cost_usd(&response.model, response.usage),
            request_id,
            latency_ms: u64::try_from(latency.as_millis()).unwrap_or(u64::MAX),
        }
    }

    fn footer(&self, ui: Ui) -> String {
        let mut facts = vec![if self.requested_model == self.model
            || self.requested_model.is_empty()
        {
            format!("model {}", self.model)
        } else {
            format!("model {} (requested {})", self.model, self.requested_model)
        }];
        facts.push(format!("{} input tokens", self.usage.input_tokens));
        facts.push(self.cost_usd.map_or_else(
            || "cost unknown".to_owned(),
            |cost| format!("est. cost ${cost:.6}"),
        ));
        facts.push(format!("{} ms", self.latency_ms));
        if let Some(request_id) = &self.request_id {
            facts.push(format!("request {request_id}"));
        }
        ui.dim(&facts.join(ui.separator()))
    }
}

impl Render for ResultEnvelope {
    fn human(&self, ui: Ui) -> String {
        let mut text = String::new();
        for (id, raw) in &self.answers {
            let answer = serde_json::from_value::<Answer>(raw.clone())
                .unwrap_or_else(|_| Answer::Unknown(raw.clone()));
            write_answer(&mut text, id, &answer, ui);
        }
        if self.answers.is_empty() {
            text.push_str("no answers\n");
        }
        text.push_str(&self.footer(ui));
        text.push('\n');
        text
    }
}

fn write_answer(text: &mut String, id: &str, answer: &Answer, ui: Ui) {
    let id = ui.bold(id);
    match answer {
        Answer::Noul(noul) => {
            let verdict = if noul.noul >= 0.5 { "yes" } else { "no" };
            let _ = writeln!(
                text,
                "{id}  {}  {}  {:.2}  {}",
                ui.dim("noul"),
                ui.bold(verdict),
                noul.noul,
                ui.bar(noul.noul, BAR_WIDTH)
            );
        }
        Answer::Choice(choice) => {
            let _ = writeln!(
                text,
                "{id}  {}  {}  {}",
                ui.dim("choice"),
                ui.bold(&choice.choice),
                ui.dim(&format!("(confidence {:.2})", choice.confidence))
            );
            let rows = descending(
                choice
                    .probabilities
                    .iter()
                    .map(|(option, p)| (option.clone(), *p, String::new())),
            );
            text.push_str(&distribution(rows, ui));
        }
        Answer::Score(score) => {
            let top = score.probabilities.len().saturating_sub(1);
            let _ = writeln!(
                text,
                "{id}  {}  {}  {}",
                ui.dim("score"),
                ui.bold(&format!("{:.2}", score.score)),
                ui.dim(&format!(
                    "(levels 0 to {top}, confidence {:.2})",
                    score.confidence
                ))
            );
            let rows = descending(score.probabilities.iter().map(|(level, p)| {
                let description = score
                    .legend
                    .get(level)
                    .map(describe_level)
                    .unwrap_or_default();
                (level.clone(), *p, description)
            }));
            text.push_str(&distribution(rows, ui));
        }
        other => {
            let kind = other.type_name().unwrap_or("unknown");
            let raw = serde_json::to_string(other).unwrap_or_default();
            let _ = writeln!(text, "{id}  {}  {raw}", ui.dim(kind));
        }
    }
}

/// Sorts by probability, highest first. The sort is stable, so ties keep the API's order.
fn descending(rows: impl Iterator<Item = (String, f64, String)>) -> Vec<(String, f64, String)> {
    let mut rows: Vec<_> = rows.collect();
    rows.sort_by(|left, right| right.1.total_cmp(&left.1));
    rows
}

fn distribution(rows: Vec<(String, f64, String)>, ui: Ui) -> String {
    let mut table = Table::default();
    for (name, probability, description) in rows {
        let bar = ui.bar(probability, BAR_WIDTH);
        let mut cells = vec![
            Cell::plain(name),
            Cell::plain(format!("{probability:.2}")),
            Cell::styled(&"x".repeat(BAR_WIDTH), bar),
        ];
        if !description.is_empty() {
            cells.push(Cell::plain(description));
        }
        table.row(cells);
    }
    table.render("    ")
}

/// A score level's description on one line: text as it is, anything structured as compact JSON.
fn describe_level(level: &jev_client::Content) -> String {
    let text = level
        .as_str()
        .map_or_else(|| level.as_value().to_string(), str::to_owned);
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::{Value, json};

    use super::ResultEnvelope;
    use crate::output::{Render, Ui};

    fn raw_response() -> Value {
        json!({
            "model": "jev-1.13.0",
            "answers": {
                "is_urgent": { "type": "noul", "noul": 0.92 },
                "department": {
                    "type": "choice", "choice": "technical", "confidence": 0.82,
                    "probabilities": { "billing": 0.08, "technical": 0.85, "sales": 0.07 }
                },
                "frustration": {
                    "type": "score", "score": 1.6, "confidence": 0.78,
                    "legend": { "0": "Calm", "1": "Frustrated", "2": { "what": "Very  angry" } },
                    "probabilities": { "0": 0.05, "1": 0.3, "2": 0.65 }
                },
                "ranking": { "type": "rank", "order": ["b", "a"], "future_field": true }
            },
            "usage": { "input_tokens": 312, "output_tokens": 48 }
        })
    }

    fn envelope() -> ResultEnvelope {
        let raw = raw_response();
        let response = serde_json::from_value(raw.clone()).unwrap();
        ResultEnvelope::new(
            &response,
            Some(&raw),
            "jev-latest",
            Some("req_1".into()),
            Duration::from_millis(905),
        )
    }

    #[test]
    fn the_json_envelope_has_the_documented_fields_and_passes_answers_through() {
        let json = serde_json::to_value(envelope()).unwrap();

        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "model",
                "requested_model",
                "answers",
                "usage",
                "cost_usd",
                "request_id",
                "latency_ms"
            ]
        );
        assert_eq!(
            json["answers"],
            raw_response()["answers"],
            "unknown types and fields survive"
        );
        assert_eq!(json["model"], "jev-1.13.0");
        assert_eq!(json["requested_model"], "jev-latest");
        assert_eq!(
            json["usage"],
            json!({ "input_tokens": 312, "output_tokens": 48 })
        );
        assert_eq!(json["latency_ms"], 905);
        assert!((json["cost_usd"].as_f64().unwrap() - 0.000_013_104).abs() < 1e-12);
    }

    #[test]
    fn an_unknown_model_has_a_null_cost_never_a_guess() {
        let mut raw = raw_response();
        raw["model"] = json!("jev-9.0.0");
        let response = serde_json::from_value(raw.clone()).unwrap();

        let envelope =
            ResultEnvelope::new(&response, Some(&raw), "jev-latest", None, Duration::ZERO);

        assert_eq!(
            serde_json::to_value(&envelope).unwrap()["cost_usd"],
            Value::Null
        );
        assert!(envelope.human(Ui::plain()).contains("cost unknown"));
    }

    #[test]
    fn without_a_raw_body_the_typed_answers_are_used() {
        let response = serde_json::from_value(raw_response()).unwrap();

        let envelope = ResultEnvelope::new(&response, None, "jev-latest", None, Duration::ZERO);

        assert_eq!(
            envelope.answers["is_urgent"],
            json!({ "type": "noul", "noul": 0.92 })
        );
    }

    #[test]
    fn the_human_form_sorts_probabilities_descending_and_shows_confidence() {
        let expected = "\
is_urgent  noul  yes  0.92  ##################..
department  choice  technical  (confidence 0.82)
    technical  0.85  #################...
    billing    0.08  ##..................
    sales      0.07  #...................
frustration  score  1.60  (levels 0 to 2, confidence 0.78)
    2  0.65  #############.......  {\"what\":\"Very angry\"}
    1  0.30  ######..............  Frustrated
    0  0.05  #...................  Calm
ranking  rank  {\"type\":\"rank\",\"order\":[\"b\",\"a\"],\"future_field\":true}
model jev-1.13.0 (requested jev-latest) | 312 input tokens | est. cost $0.000013 | 905 ms | request req_1
";

        assert_eq!(envelope().human(Ui::plain()), expected);
    }

    #[test]
    fn the_human_form_uses_unicode_and_colour_when_the_terminal_allows() {
        let text = envelope().human(Ui::new(true, true));

        assert!(text.contains('█') && text.contains(" · "), "{text}");
        assert!(text.contains("\u{1b}["), "{text:?}");
    }
}
