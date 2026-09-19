//! `--field`: print one value from a command's output, raw.

use serde_json::Value;

use crate::error::CliError;

/// One step of a field path.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Key(String),
    Index(usize),
}

/// Parses `answers.is_urgent.noul`, `models[0].name` or `models.0.name`. A literal dot inside a
/// key is written `\.`.
fn parse(path: &str) -> Result<Vec<Step>, CliError> {
    let invalid = |problem: &str| {
        CliError::usage(format!("--field `{path}` is not a valid path: {problem}"))
            .hint("write a dot-separated path such as `answers.is_urgent.noul` or `models[0].name`")
    };

    let mut steps = Vec::new();
    let mut key = String::new();
    let mut characters = path.chars();
    let mut expect_separator = false;
    while let Some(character) = characters.next() {
        match character {
            '\\' => key.push(
                characters
                    .next()
                    .ok_or_else(|| invalid("it ends with a backslash"))?,
            ),
            '.' => {
                if key.is_empty() && !expect_separator {
                    return Err(invalid("it has an empty segment"));
                }
                push_key(&mut steps, &mut key);
                expect_separator = false;
            }
            '[' => {
                push_key(&mut steps, &mut key);
                let index: String = characters
                    .by_ref()
                    .take_while(|next| *next != ']')
                    .collect();
                let index = index
                    .trim()
                    .parse()
                    .map_err(|_| invalid("an index must be a whole number"))?;
                steps.push(Step::Index(index));
                expect_separator = true;
            }
            _ if expect_separator => return Err(invalid("a `]` must be followed by `.` or `[`")),
            other => key.push(other),
        }
    }
    if key.is_empty() && !expect_separator {
        return Err(invalid("it has an empty segment"));
    }
    push_key(&mut steps, &mut key);
    Ok(steps)
}

fn push_key(steps: &mut Vec<Step>, key: &mut String) {
    if !key.is_empty() {
        steps.push(Step::Key(std::mem::take(key)));
    }
}

/// Finds the value at `path`.
///
/// # Errors
///
/// A usage error when the path is malformed or leads nowhere. The message says where the path
/// stopped matching and what was available there, so the caller can correct it in one step.
pub(crate) fn select<'a>(value: &'a Value, path: &str) -> Result<&'a Value, CliError> {
    let mut current = value;
    let mut walked = String::new();
    for step in parse(path)? {
        let next = match (&step, current) {
            (Step::Key(key), Value::Object(map)) => map.get(key),
            // A numeric key also indexes an array, so `models.0.name` works like `models[0].name`.
            (Step::Key(key), Value::Array(items)) => {
                key.parse::<usize>().ok().and_then(|index| items.get(index))
            }
            (Step::Index(index), Value::Array(items)) => items.get(*index),
            _ => None,
        };
        let Some(next) = next else {
            let wanted = match &step {
                Step::Key(key) => format!("`{key}`"),
                Step::Index(index) => format!("index {index}"),
            };
            let at = if walked.is_empty() {
                "the top level".to_owned()
            } else {
                format!("`{walked}`")
            };
            return Err(
                CliError::usage(format!("--field `{path}`: there is no {wanted} at {at}"))
                    .hint(available(current)),
            );
        };
        if !walked.is_empty() {
            walked.push('.');
        }
        walked.push_str(&match step {
            Step::Key(key) => key,
            Step::Index(index) => index.to_string(),
        });
        current = next;
    }
    Ok(current)
}

/// What could have been asked for at this point, as a hint.
fn available(value: &Value) -> String {
    match value {
        Value::Object(map) if map.is_empty() => "that object is empty".to_owned(),
        Value::Object(map) => {
            let keys: Vec<&str> = map.keys().map(String::as_str).take(20).collect();
            format!("available there: {}", keys.join(", "))
        }
        Value::Array(items) if items.is_empty() => "that array is empty".to_owned(),
        Value::Array(items) => format!(
            "that is an array; use an index from 0 to {}",
            items.len() - 1
        ),
        _ => "that is a single value with nothing inside it".to_owned(),
    }
}

/// A value as `--field` prints it: text without quotes, anything structured as compact JSON.
pub(crate) fn to_raw(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{select, to_raw};

    fn sample() -> serde_json::Value {
        json!({
            "model": "jev-1.13.0",
            "answers": { "is_urgent": { "type": "noul", "noul": 0.92 }, "a.b": { "noul": 0.5 } },
            "models": [{ "name": "jev-latest" }, { "name": "jev-preview" }],
            "request_id": null,
            "empty": {}
        })
    }

    #[test]
    fn selects_nested_values_and_prints_them_raw() {
        let value = sample();

        assert_eq!(
            to_raw(select(&value, "answers.is_urgent.noul").unwrap()),
            "0.92"
        );
        assert_eq!(to_raw(select(&value, "model").unwrap()), "jev-1.13.0");
        assert_eq!(to_raw(select(&value, "request_id").unwrap()), "null");
        assert_eq!(
            to_raw(select(&value, "answers.is_urgent").unwrap()),
            r#"{"type":"noul","noul":0.92}"#
        );
    }

    #[test]
    fn indexes_arrays_both_ways_and_escapes_dots_in_keys() {
        let value = sample();

        assert_eq!(
            to_raw(select(&value, "models[1].name").unwrap()),
            "jev-preview"
        );
        assert_eq!(
            to_raw(select(&value, "models.0.name").unwrap()),
            "jev-latest"
        );
        assert_eq!(to_raw(select(&value, r"answers.a\.b.noul").unwrap()), "0.5");
    }

    #[test]
    fn a_missing_path_says_where_it_stopped_and_what_was_there() {
        let value = sample();

        let error = select(&value, "answers.is_urgnt.noul").unwrap_err();
        assert_eq!(error.exit.code(), 2);
        assert_eq!(
            error.message,
            "--field `answers.is_urgnt.noul`: there is no `is_urgnt` at `answers`"
        );
        assert_eq!(
            error.hint.as_deref(),
            Some("available there: is_urgent, a.b")
        );

        let hints: Vec<String> = ["nope", "models[5]", "model.x", "empty.x"]
            .iter()
            .map(|path| select(&value, path).unwrap_err().hint.unwrap())
            .collect();
        assert_eq!(
            hints,
            [
                "available there: model, answers, models, request_id, empty",
                "that is an array; use an index from 0 to 1",
                "that is a single value with nothing inside it",
                "that object is empty",
            ]
        );
    }

    #[test]
    fn malformed_paths_are_usage_errors() {
        for path in [
            "",
            ".",
            "a..b",
            "a.",
            ".a",
            "models[x]",
            "models[0]name",
            "a\\",
        ] {
            let error = select(&sample(), path).unwrap_err();

            assert_eq!(error.exit.code(), 2, "{path:?}");
            assert!(
                error.message.contains("not a valid path"),
                "{path:?}: {}",
                error.message
            );
        }
    }
}
