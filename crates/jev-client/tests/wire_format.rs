//! The typed model against real payloads: the upstream API reference examples, and payloads
//! captured from the live API.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use jev_client::{Answer, ModelList, Request, Response, pricing};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// Parses `json` as `T` and serialises it again.
fn round_trip<T: DeserializeOwned + Serialize>(name: &str, json: &Value) -> Value {
    let typed: T = serde_json::from_value(json.clone())
        .unwrap_or_else(|error| panic!("{name} does not deserialise: {error}"));
    serde_json::to_value(&typed).unwrap()
}

/// Asserts equality of content *and* of key order, which `Value` equality alone ignores.
fn assert_same_json(name: &str, actual: &Value, expected: &Value) {
    assert_eq!(actual, expected, "{name} changed in a round trip");
    assert_eq!(
        actual.to_string(),
        expected.to_string(),
        "{name} changed key order"
    );
}

#[test]
fn every_api_reference_request_round_trips() {
    let fixtures = common::load("api-reference", "request-");
    assert_eq!(
        fixtures.len(),
        4,
        "one plain Noul, one Noul with criteria, a Choice and a Score"
    );

    for fixture in fixtures {
        let again = round_trip::<Request>(&fixture.name, &fixture.json);
        assert_same_json(&fixture.name, &again, &fixture.json);
    }
}

#[test]
fn every_api_reference_response_round_trips_into_a_known_answer() {
    let fixtures = common::load("api-reference", "response-");
    assert_eq!(fixtures.len(), 3, "one response per answer type");

    for fixture in fixtures {
        let again = round_trip::<Response>(&fixture.name, &fixture.json);
        assert_same_json(&fixture.name, &again, &fixture.json);

        let response: Response = serde_json::from_value(fixture.json).unwrap();
        for (id, answer) in &response.answers {
            assert!(
                !matches!(answer, Answer::Unknown(_)),
                "{}: `{id}` is unknown",
                fixture.name
            );
        }
    }
}

#[test]
fn a_live_request_with_structured_content_round_trips() {
    for fixture in common::load("live", "request-") {
        let again = round_trip::<Request>(&fixture.name, &fixture.json);
        assert_same_json(&fixture.name, &again, &fixture.json);
    }
}

#[test]
fn a_live_response_parses_with_a_structured_legend_and_a_resolved_model() {
    for fixture in common::load("live", "response-") {
        // Equivalent JSON, not identical text: the live server orders an answer's fields
        // differently from the API reference, and the typed answers follow the reference.
        let again = round_trip::<Response>(&fixture.name, &fixture.json);
        assert_eq!(
            again, fixture.json,
            "{} changed in a round trip",
            fixture.name
        );

        let response: Response = serde_json::from_value(fixture.json).unwrap();
        assert_eq!(
            response.model, "jev-1.13.0",
            "the server reports the versioned id"
        );
        assert!(pricing::estimate_cost_usd(&response.model, response.usage).is_some());

        let noul = response
            .answers
            .get("is_urgent")
            .and_then(Answer::as_noul)
            .unwrap();
        assert!((0.0..=1.0).contains(&noul.noul));

        let choice = response
            .answers
            .get("department")
            .and_then(Answer::as_choice)
            .unwrap();
        assert!(choice.probabilities.contains_key(&choice.choice));
        let total: f64 = choice.probabilities.values().sum();
        assert!((total - 1.0).abs() < 0.02, "probabilities sum to {total}");

        let score = response
            .answers
            .get("frustration")
            .and_then(Answer::as_score)
            .unwrap();
        assert_eq!(score.legend.keys().collect::<Vec<_>>(), ["0", "1", "2"]);
        assert!(
            score
                .legend
                .values()
                .any(|level| level.as_value().is_object())
        );
        assert!(
            score
                .legend
                .values()
                .any(|level| level.as_value().is_array())
        );
    }
}

#[test]
fn the_live_models_listing_parses() {
    for fixture in common::load("live", "models") {
        let again = round_trip::<ModelList>(&fixture.name, &fixture.json);
        assert_same_json(&fixture.name, &again, &fixture.json);

        let list: ModelList = serde_json::from_value(fixture.json).unwrap();
        assert!(list.models.iter().any(|model| model.name == "jev-latest"));
        assert!(list.models.iter().all(|model| model.release_date.is_some()));
    }
}

#[test]
fn an_extra_top_level_field_and_an_unknown_answer_type_do_not_fail_the_response() {
    let unknown = json!({ "type": "rank", "order": ["billing", "sales"], "confidence": 0.7 });
    let mut json = common::load("api-reference", "response-choice")
        .remove(0)
        .json;
    json["served_by"] = json!("eu-west-1");
    json["answers"]["ranking"] = unknown.clone();

    let response: Response = serde_json::from_value(json).unwrap();

    assert_eq!(response.answers.len(), 2);
    assert!(
        response
            .answers
            .get("department")
            .and_then(Answer::as_choice)
            .is_some()
    );
    let ranking = response.answers.get("ranking").unwrap();
    assert_eq!(ranking, &Answer::Unknown(unknown.clone()));
    assert_eq!(ranking.type_name(), Some("rank"));
    assert_eq!(
        serde_json::to_value(ranking).unwrap(),
        unknown,
        "raw JSON is passed through"
    );
}
