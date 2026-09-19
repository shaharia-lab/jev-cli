//! The JSON Schemas derived from the types, checked with an independent validator.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use jev_client::{ModelList, Request, Response};
use jsonschema::Validator;
use schemars::{JsonSchema, schema_for};
use serde_json::{Value, json};

/// Builds a validator for `T`'s schema, after checking the schema is itself a valid schema.
fn validator_for<T: JsonSchema>() -> Validator {
    let schema = schema_for!(T).to_value();
    assert_eq!(
        schema.get("$schema").and_then(Value::as_str),
        Some("https://json-schema.org/draft/2020-12/schema")
    );
    assert!(
        jsonschema::meta::is_valid(&schema),
        "not a valid JSON Schema: {schema}"
    );
    jsonschema::validator_for(&schema).unwrap()
}

fn errors(validator: &Validator, instance: &Value) -> Vec<String> {
    validator
        .iter_errors(instance)
        .map(|error| error.to_string())
        .collect()
}

fn request_with(question: &Value) -> Value {
    json!({ "state": "s", "model": "jev-latest", "questions": { "q": question } })
}

#[test]
fn the_request_schema_accepts_every_example_request() {
    let validator = validator_for::<Request>();

    let fixtures = [
        common::load("api-reference", "request-"),
        common::load("live", "request-"),
    ];
    for fixture in fixtures.into_iter().flatten() {
        let errors = errors(&validator, &fixture.json);
        assert!(
            errors.is_empty(),
            "{} was rejected: {errors:?}",
            fixture.name
        );
    }
}

#[test]
fn the_request_schema_rejects_a_question_with_no_type() {
    let validator = validator_for::<Request>();
    let untyped = request_with(&json!({ "instructions": "Does this convey urgency?" }));
    let typed =
        request_with(&json!({ "type": "noul", "instructions": "Does this convey urgency?" }));

    assert!(validator.is_valid(&typed));
    assert!(!validator.is_valid(&untyped));
}

#[test]
fn the_request_schema_rejects_malformed_requests() {
    let validator = validator_for::<Request>();
    let noul = json!({ "type": "noul", "instructions": "?" });
    let rejected = [
        (
            "an unknown question type",
            request_with(&json!({ "type": "rank", "instructions": "?" })),
        ),
        (
            "a choice without criteria",
            request_with(&json!({ "type": "choice", "instructions": "?" })),
        ),
        (
            "a score whose criteria is a map",
            request_with(
                &json!({ "type": "score", "instructions": "?", "criteria": { "a": "b" } }),
            ),
        ),
        (
            "a null score level",
            request_with(
                &json!({ "type": "score", "instructions": "?", "criteria": ["low", null] }),
            ),
        ),
        (
            "a numeric choice description",
            request_with(&json!({ "type": "choice", "instructions": "?", "criteria": { "a": 1 } })),
        ),
        (
            "a question without instructions",
            request_with(&json!({ "type": "noul" })),
        ),
        (
            "an unknown top-level field",
            json!({ "state": "s", "model": "m", "questions": { "q": noul }, "temperature": 0 }),
        ),
        (
            "a misspelt criteria key",
            request_with(&json!({
                "type": "choice", "instructions": "?", "criteria": { "a": null }, "critera": {}
            })),
        ),
        (
            "a noul criteria side other than true or false",
            request_with(&json!({
                "type": "noul", "instructions": "?", "criteria": { "true": "y", "maybe": "m" }
            })),
        ),
        (
            "a null state",
            json!({ "state": null, "model": "m", "questions": { "q": noul } }),
        ),
        (
            "a numeric state",
            json!({ "state": 42, "model": "m", "questions": { "q": noul } }),
        ),
        (
            "a missing model",
            json!({ "state": "s", "questions": { "q": noul } }),
        ),
        (
            "a missing questions map",
            json!({ "state": "s", "model": "m" }),
        ),
    ];

    for (what, instance) in rejected {
        assert!(!validator.is_valid(&instance), "the schema accepted {what}");
    }
}

#[test]
fn the_request_schema_accepts_null_where_the_api_does() {
    let validator = validator_for::<Request>();
    let accepted = [
        request_with(
            &json!({ "type": "choice", "instructions": "?", "criteria": { "a": null, "b": null } }),
        ),
        request_with(&json!({ "type": "noul", "instructions": "?", "criteria": { "true": null } })),
        request_with(
            &json!({ "type": "noul", "instructions": null, "criteria": { "true": "yes" } }),
        ),
    ];

    for instance in accepted {
        let errors = errors(&validator, &instance);
        assert!(errors.is_empty(), "{instance} was rejected: {errors:?}");
    }
}

#[test]
fn the_response_schema_accepts_known_and_unknown_answers() {
    let validator = validator_for::<Response>();

    let fixtures = [
        common::load("api-reference", "response-"),
        common::load("live", "response-"),
    ];
    for fixture in fixtures.into_iter().flatten() {
        let errors = errors(&validator, &fixture.json);
        assert!(
            errors.is_empty(),
            "{} was rejected: {errors:?}",
            fixture.name
        );
    }

    let newer = json!({
        "model": "jev-2.0.0",
        "answers": { "q": { "type": "rank", "order": ["a"] } },
        "usage": { "input_tokens": 1, "output_tokens": 0 }
    });
    assert!(
        validator.is_valid(&newer),
        "an unknown answer type must stay valid"
    );
}

#[test]
fn the_models_schema_accepts_the_live_listing() {
    let validator = validator_for::<ModelList>();

    for fixture in common::load("live", "models") {
        let errors = errors(&validator, &fixture.json);
        assert!(
            errors.is_empty(),
            "{} was rejected: {errors:?}",
            fixture.name
        );
    }
}

#[test]
fn schemas_carry_the_documentation_of_the_types() {
    let schema = schema_for!(Request).to_value();

    let state = schema
        .pointer("/properties/state/description")
        .and_then(Value::as_str);
    let model = schema
        .pointer("/properties/model/description")
        .and_then(Value::as_str);

    assert!(state.is_some_and(|text| !text.is_empty()), "{schema}");
    assert!(
        model.is_some_and(|text| text.contains("jev-latest")),
        "{schema}"
    );
}

/// Every `description` string anywhere inside `schema`, with the JSON pointer it was found at.
fn descriptions(schema: &Value, pointer: &str, found: &mut Vec<(String, String)>) {
    match schema {
        Value::Object(map) => {
            if let Some(Value::String(text)) = map.get("description") {
                found.push((pointer.to_owned(), text.clone()));
            }
            for (key, value) in map {
                descriptions(value, &format!("{pointer}/{key}"), found);
            }
        }
        Value::Array(items) => {
            for (index, value) in items.iter().enumerate() {
                descriptions(value, &format!("{pointer}/{index}"), found);
            }
        }
        _ => {}
    }
}

#[test]
fn schema_descriptions_are_written_for_readers_of_the_schema_not_of_rustdoc() {
    let schemas = json!([
        schema_for!(Request).to_value(),
        schema_for!(Response).to_value(),
        schema_for!(ModelList).to_value(),
    ]);
    let mut found = Vec::new();
    descriptions(&schemas, "", &mut found);
    assert!(
        found.len() > 20,
        "expected the doc comments to be carried into the schemas"
    );

    for (pointer, text) in found {
        for rustdoc_syntax in ["```", "](", "[`", "crate::"] {
            assert!(
                !text.contains(rustdoc_syntax),
                "{pointer} leaks rustdoc syntax {rustdoc_syntax:?}; add a #[schemars(description)]: {text}"
            );
        }
    }
}
