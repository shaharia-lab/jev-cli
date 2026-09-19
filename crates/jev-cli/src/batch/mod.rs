//! The batch engine: one question set, many rows, one API call per row.
//!
//! It knows nothing of `clap` or of the terminal, so that `jev batch run` and the MCP server's
//! `batch_run` tool drive the same code. Every row goes through [`crate::evaluate::prepare`] and
//! [`crate::evaluate::send`], exactly like `jev eval`.
//!
//! Memory stays proportional to the concurrency, not to the input: rows are read one at a time on
//! a thread of their own, handed over through a channel that holds at most one row per worker, and
//! each record is written as soon as its row finishes. Only the ids of the rows are remembered,
//! as 16-byte fingerprints, and only when an `--id-field` makes repeats possible.

mod engine;
mod input;
mod record;
mod summary;

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};

use serde_json::{Map, Value, json};

pub(crate) use engine::{Job, run};
pub(crate) use input::{Row, RowFormat, RowProblem, RowSource, Rows};
pub(crate) use record::Record;
pub(crate) use summary::Summary;

use crate::error::CliError;

/// How many problems an error spells out before counting the rest.
const MAX_LISTED_PROBLEMS: usize = 8;

/// What each row sends as its state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StateMapping {
    /// The whole row.
    WholeRow,
    /// The value of one field.
    Field(String),
    /// An object of only these fields, in this order.
    Fields(Vec<String>),
}

impl StateMapping {
    /// The fields a row must have.
    fn required(&self) -> &[String] {
        match self {
            Self::WholeRow => &[],
            Self::Field(field) => std::slice::from_ref(field),
            Self::Fields(fields) => fields,
        }
    }
}

/// How a row becomes an id and a state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Mapping {
    pub(crate) state: StateMapping,
    /// The field that identifies a row; the line number when there is none.
    pub(crate) id_field: Option<String>,
}

/// A row ready to be sent.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Keyed {
    /// The id as it appears in the record: the field's string or number, or the line number.
    pub(crate) id: Value,
    pub(crate) state: Value,
}

impl Mapping {
    /// Checks that a CSV header has every column the mapping names, before any row is read.
    ///
    /// # Errors
    ///
    /// A usage error naming the missing columns and listing the ones there are.
    pub(crate) fn check_columns(&self, columns: &[String]) -> Result<(), CliError> {
        let missing: Vec<&str> = self
            .state
            .required()
            .iter()
            .chain(&self.id_field)
            .filter(|field| !columns.contains(field))
            .map(String::as_str)
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        Err(CliError::usage(format!(
            "the input has no column {}; nothing was sent",
            quoted(&missing)
        ))
        .hint(format!(
            "the columns are {}; check --state-field, --state-fields and --id-field",
            quoted(&columns.iter().map(String::as_str).collect::<Vec<_>>())
        )))
    }

    /// Finds a row's id and builds its state.
    ///
    /// # Errors
    ///
    /// A problem naming the line and the field, never the row's content.
    pub(crate) fn key(&self, row: Row) -> Result<Keyed, RowProblem> {
        let Row { line, value } = row;
        let id = match &self.id_field {
            None => json!(line),
            Some(field) => match field_of(&value, field, line)? {
                Value::String(text) if !text.trim().is_empty() => Value::String(text.clone()),
                Value::Number(number) => Value::Number(number.clone()),
                _ => {
                    return Err(RowProblem::at(
                        line,
                        format!("the id field `{field}` must be a non-empty string or a number"),
                    ));
                }
            },
        };
        let state = match &self.state {
            StateMapping::WholeRow => value,
            StateMapping::Field(field) => match field_of(&value, field, line)? {
                Value::Null => {
                    return Err(RowProblem::at(
                        line,
                        format!("the state field `{field}` is null"),
                    ));
                }
                state => state.clone(),
            },
            StateMapping::Fields(fields) => {
                let mut trimmed = Map::new();
                for field in fields {
                    trimmed.insert(field.clone(), field_of(&value, field, line)?.clone());
                }
                Value::Object(trimmed)
            }
        };
        Ok(Keyed { id, state })
    }
}

fn field_of<'a>(row: &'a Value, field: &str, line: u64) -> Result<&'a Value, RowProblem> {
    match row {
        Value::Object(fields) => fields
            .get(field)
            .ok_or_else(|| RowProblem::at(line, format!("the row has no field `{field}`"))),
        _ => Err(RowProblem::at(
            line,
            format!("the row is not an object, so it has no field `{field}`"),
        )),
    }
}

fn quoted(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The ids seen so far, as fingerprints: a million ids take about 32 MB, whatever their length.
#[derive(Debug, Default)]
pub(crate) struct Ids(HashSet<u128>);

impl Ids {
    /// Records an id.
    ///
    /// # Errors
    ///
    /// A problem when the id has been seen before. `"7"` and `7` are the same id.
    pub(crate) fn insert(&mut self, id: &Value, line: u64) -> Result<(), RowProblem> {
        let text = match id {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        let fingerprint = [0_u8, 1].map(|seed| {
            let mut hasher = DefaultHasher::new();
            seed.hash(&mut hasher);
            text.hash(&mut hasher);
            hasher.finish()
        });
        let [high, low] = fingerprint;
        if self.0.insert((u128::from(high) << 64) | u128::from(low)) {
            Ok(())
        } else {
            Err(RowProblem::at(
                line,
                format!("the id `{text}` is used by an earlier row; ids must be unique"),
            ))
        }
    }
}

/// Reads every row of a rereadable input once, before anything is sent, and checks that each can
/// be mapped and that no id repeats.
///
/// Returns the number of rows.
///
/// # Errors
///
/// A usage error listing the problems found. Nothing has been sent.
pub(crate) fn preflight(rows: Rows, mapping: &Mapping) -> Result<u64, CliError> {
    let mut ids = mapping.id_field.is_some().then(Ids::default);
    let mut count = 0_u64;
    let mut problems = Vec::new();
    for row in rows {
        let checked = row.and_then(|row| {
            let line = row.line;
            let keyed = mapping.key(row)?;
            match &mut ids {
                Some(ids) => ids.insert(&keyed.id, line),
                None => Ok(()),
            }
        });
        count += 1;
        if let Err(problem) = checked {
            problems.push(problem);
        }
    }
    if problems.is_empty() {
        Ok(count)
    } else {
        Err(invalid_input(&problems))
    }
}

/// The error for rows that cannot be used.
pub(crate) fn invalid_input(problems: &[RowProblem]) -> CliError {
    let count = problems.len();
    let mut error = CliError::usage(format!(
        "the input has {count} row{} that cannot be used; nothing was sent",
        if count == 1 { "" } else { "s" }
    ))
    .hint("fix the input, or the --state-field, --state-fields or --id-field that name its fields");
    for problem in problems.iter().take(MAX_LISTED_PROBLEMS) {
        error.lines.push(format!("- {}", problem.describe()));
    }
    if count > MAX_LISTED_PROBLEMS {
        error
            .lines
            .push(format!("- and {} more", count - MAX_LISTED_PROBLEMS));
    }
    let listed: Vec<Value> = problems
        .iter()
        .take(100)
        .map(|problem| json!({ "line": problem.line, "message": problem.message }))
        .collect();
    error.details = Some(json!({ "problems": listed, "count": count }));
    error
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Ids, Mapping, Row, RowFormat, Rows, StateMapping, preflight};

    fn row(line: u64, value: serde_json::Value) -> Row {
        Row { line, value }
    }

    fn mapping(state: StateMapping, id_field: Option<&str>) -> Mapping {
        Mapping {
            state,
            id_field: id_field.map(str::to_owned),
        }
    }

    #[test]
    fn the_state_is_the_row_one_field_or_a_trimmed_object() {
        let ticket = json!({ "id": "t-1", "subject": "Refund", "body": "Please", "internal": "x" });

        let whole = mapping(StateMapping::WholeRow, None)
            .key(row(3, ticket.clone()))
            .unwrap();
        let field = mapping(StateMapping::Field("body".into()), Some("id"))
            .key(row(3, ticket.clone()))
            .unwrap();
        let fields = mapping(
            StateMapping::Fields(vec!["subject".into(), "body".into()]),
            None,
        )
        .key(row(3, ticket.clone()))
        .unwrap();

        assert_eq!((whole.id, whole.state), (json!(3), ticket));
        assert_eq!((field.id, field.state), (json!("t-1"), json!("Please")));
        assert_eq!(
            serde_json::to_string(&fields.state).unwrap(),
            r#"{"subject":"Refund","body":"Please"}"#
        );
    }

    #[test]
    fn a_missing_field_or_an_unusable_id_is_a_problem_on_its_line() {
        let body = mapping(StateMapping::Field("body".into()), None);
        let by_id = mapping(StateMapping::WholeRow, Some("id"));

        let cases = [
            (
                body.key(row(4, json!({ "text": "x" }))),
                "the row has no field `body`",
            ),
            (
                body.key(row(4, json!("plain"))),
                "the row is not an object, so it has no field `body`",
            ),
            (
                body.key(row(4, json!({ "body": null }))),
                "the state field `body` is null",
            ),
            (
                by_id.key(row(4, json!({ "id": ["a"] }))),
                "the id field `id` must be a non-empty string or a number",
            ),
            (
                by_id.key(row(4, json!({ "id": " " }))),
                "the id field `id` must be a non-empty string or a number",
            ),
        ];
        for (result, message) in cases {
            let problem = result.unwrap_err();
            assert_eq!((problem.line, problem.message.as_str()), (Some(4), message));
        }
    }

    #[test]
    fn an_id_repeats_whether_it_is_written_as_a_string_or_a_number() {
        let mut ids = Ids::default();

        assert!(ids.insert(&json!("a"), 1).is_ok());
        assert!(ids.insert(&json!(7), 2).is_ok());
        assert!(ids.insert(&json!("b"), 3).is_ok());
        let repeated = ids.insert(&json!("7"), 4).unwrap_err();

        assert_eq!(
            repeated.describe(),
            "line 4: the id `7` is used by an earlier row; ids must be unique"
        );
    }

    #[test]
    fn preflight_counts_the_rows_and_lists_every_problem() {
        let text =
            "{\"id\": 1, \"body\": \"a\"}\n{\"id\": 2}\n{\"id\": 1, \"body\": \"c\"}\nnot json\n";
        let rows = || Rows::new(Box::new(text.as_bytes()), RowFormat::Jsonl).unwrap();

        let error = preflight(
            rows(),
            &mapping(StateMapping::Field("body".into()), Some("id")),
        )
        .unwrap_err();
        let fine = preflight(
            Rows::new(Box::new("{}\n{}\n".as_bytes()), RowFormat::Jsonl).unwrap(),
            &mapping(StateMapping::WholeRow, None),
        );

        assert_eq!(error.exit.code(), 2);
        assert_eq!(
            error.message,
            "the input has 3 rows that cannot be used; nothing was sent"
        );
        assert_eq!(
            error.lines,
            [
                "- line 2: the row has no field `body`",
                "- line 3: the id `1` is used by an earlier row; ids must be unique",
                "- line 4: not valid JSON: expected ident at line 1 column 2",
            ]
        );
        assert_eq!(error.details.unwrap()["count"], 3);
        assert_eq!(fine.unwrap(), 2);
    }

    #[test]
    fn a_csv_header_without_a_named_column_is_refused_at_once() {
        let rows = Rows::new(Box::new("id,subject\n".as_bytes()), RowFormat::Csv).unwrap();
        let columns = rows.columns().unwrap();

        let error = mapping(StateMapping::Field("body".into()), Some("ticket"))
            .check_columns(columns)
            .unwrap_err();

        assert_eq!(
            error.message,
            "the input has no column `body`, `ticket`; nothing was sent"
        );
        assert!(
            error
                .hint
                .unwrap()
                .starts_with("the columns are `id`, `subject`")
        );
    }
}
