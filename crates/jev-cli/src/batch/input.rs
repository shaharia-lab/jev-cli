//! Reading the rows of a batch, one at a time, from a JSONL or CSV stream.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

use clap::ValueEnum;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::error::CliError;

/// The format of a batch's input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum RowFormat {
    /// One JSON value per line
    Jsonl,
    /// Comma-separated values with a header row
    Csv,
}

impl RowFormat {
    /// The format of a file, by its extension: `.csv` is CSV, anything else JSONL.
    pub(crate) fn of_path(path: &Path) -> Self {
        match path.extension().and_then(|extension| extension.to_str()) {
            Some(extension) if extension.eq_ignore_ascii_case("csv") => Self::Csv,
            _ => Self::Jsonl,
        }
    }
}

/// Where the rows come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RowSource {
    File(String),
    Stdin,
}

impl RowSource {
    /// Whether the input can be read a second time, which decides whether it is checked in full
    /// before anything is sent.
    pub(crate) const fn is_rereadable(&self) -> bool {
        matches!(self, Self::File(_))
    }

    /// Opens the input.
    ///
    /// # Errors
    ///
    /// A usage error when the file cannot be opened.
    pub(crate) fn open(&self, format: RowFormat) -> Result<Rows, CliError> {
        let reader: Box<dyn Read + Send> = match self {
            Self::File(path) => Box::new(File::open(path).map_err(|error| {
                CliError::usage(format!("cannot read the input file {path}: {error}"))
                    .hint("check the path passed to --input")
            })?),
            Self::Stdin => Box::new(io::stdin()),
        };
        Rows::new(reader, format)
    }
}

/// One row of the input, before it is mapped to an id and a state.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    /// The 1-based line the row starts on.
    pub(crate) line: u64,
    /// The row: any JSON value for JSONL, an object of strings for CSV.
    pub(crate) value: Value,
}

/// A row that cannot be used. The message never quotes the row, which may hold customer data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct RowProblem {
    /// The line it is on, when it is known.
    pub(crate) line: Option<u64>,
    pub(crate) message: String,
}

impl RowProblem {
    pub(crate) fn at(line: u64, message: impl Into<String>) -> Self {
        Self {
            line: Some(line),
            message: message.into(),
        }
    }

    /// The problem as one line for a person.
    pub(crate) fn describe(&self) -> String {
        match self.line {
            Some(line) => format!("line {line}: {}", self.message),
            None => self.message.clone(),
        }
    }
}

/// The rows of an input, read lazily: only the row being handed out is in memory.
pub(crate) struct Rows {
    inner: Inner,
    /// The CSV header, which says which fields every row has.
    columns: Option<Vec<String>>,
    finished: bool,
}

enum Inner {
    Jsonl {
        reader: BufReader<Box<dyn Read + Send>>,
        line: u64,
        buffer: Vec<u8>,
    },
    Csv {
        reader: csv::Reader<Box<dyn Read + Send>>,
        record: csv::StringRecord,
    },
}

impl Rows {
    /// Starts reading. A CSV header is read at once, so that a missing column is known before
    /// any row is.
    ///
    /// # Errors
    ///
    /// A usage error when a CSV header is missing, unreadable or names a column twice.
    pub(crate) fn new(reader: Box<dyn Read + Send>, format: RowFormat) -> Result<Self, CliError> {
        match format {
            RowFormat::Jsonl => Ok(Self {
                inner: Inner::Jsonl {
                    reader: BufReader::new(reader),
                    line: 0,
                    buffer: Vec::new(),
                },
                columns: None,
                finished: false,
            }),
            RowFormat::Csv => {
                let mut reader = csv::ReaderBuilder::new()
                    .has_headers(true)
                    .from_reader(reader);
                let header = reader.headers().map_err(|error| {
                    CliError::usage(format!(
                        "the CSV header cannot be read: {}",
                        csv_problem(&error)
                    ))
                    .hint("the first line of a CSV input names its columns")
                })?;
                let columns: Vec<String> = header.iter().map(str::to_owned).collect();
                for (index, column) in columns.iter().enumerate() {
                    if columns.iter().take(index).any(|earlier| earlier == column) {
                        return Err(CliError::usage(format!(
                            "the CSV header names the column `{column}` twice"
                        ))
                        .hint("give every column a different name"));
                    }
                }
                Ok(Self {
                    inner: Inner::Csv {
                        reader,
                        record: csv::StringRecord::new(),
                    },
                    columns: Some(columns),
                    finished: false,
                })
            }
        }
    }

    /// The columns of a CSV input; `None` for JSONL, whose rows each have their own fields.
    pub(crate) fn columns(&self) -> Option<&[String]> {
        self.columns.as_deref()
    }
}

impl Iterator for Rows {
    type Item = Result<Row, RowProblem>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        let item = match &mut self.inner {
            Inner::Jsonl {
                reader,
                line,
                buffer,
            } => next_jsonl(reader, line, buffer),
            Inner::Csv { reader, record } => next_csv(reader, record, self.columns.as_deref()),
        };
        // A read error cannot be skipped past: the stream is in an unknown state.
        if matches!(item, None | Some(Err(RowProblem { line: None, .. }))) {
            self.finished = true;
        }
        item
    }
}

fn next_jsonl(
    reader: &mut BufReader<Box<dyn Read + Send>>,
    line: &mut u64,
    buffer: &mut Vec<u8>,
) -> Option<Result<Row, RowProblem>> {
    loop {
        buffer.clear();
        match reader.read_until(b'\n', buffer) {
            Ok(0) => return None,
            Ok(_) => {}
            Err(error) => {
                return Some(Err(RowProblem {
                    line: None,
                    message: format!("the input cannot be read after line {line}: {error}"),
                }));
            }
        }
        *line += 1;
        let mut bytes = buffer.as_slice();
        if *line == 1 {
            bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        }
        if bytes.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        return Some(
            serde_json::from_slice::<Value>(bytes)
                .map(|value| Row { line: *line, value })
                // serde_json reports a position and a category, never the text itself.
                .map_err(|error| RowProblem::at(*line, format!("not valid JSON: {error}"))),
        );
    }
}

fn next_csv(
    reader: &mut csv::Reader<Box<dyn Read + Send>>,
    record: &mut csv::StringRecord,
    columns: Option<&[String]>,
) -> Option<Result<Row, RowProblem>> {
    match reader.read_record(record) {
        Ok(false) => None,
        Ok(true) => {
            let line = record.position().map_or(0, csv::Position::line);
            let row: Map<String, Value> = columns
                .unwrap_or_default()
                .iter()
                .zip(record.iter())
                .map(|(column, cell)| (column.clone(), Value::String(cell.to_owned())))
                .collect();
            Some(Ok(Row {
                line,
                value: Value::Object(row),
            }))
        }
        Err(error) => {
            let line = error.position().map(csv::Position::line);
            let problem = csv_problem(&error);
            Some(Err(match (line, error.is_io_error()) {
                (Some(line), false) => RowProblem::at(line, problem),
                _ => RowProblem {
                    line: None,
                    message: format!("the input cannot be read: {problem}"),
                },
            }))
        }
    }
}

/// A CSV error in words, without the position the caller already reports and without any content.
fn csv_problem(error: &csv::Error) -> String {
    match error.kind() {
        csv::ErrorKind::UnequalLengths {
            expected_len, len, ..
        } => format!("has {len} fields where the header has {expected_len}"),
        csv::ErrorKind::Utf8 { .. } => "is not valid UTF-8".to_owned(),
        csv::ErrorKind::Io(error) => error.to_string(),
        _ => "is not valid CSV".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Row, RowFormat, RowProblem, Rows};

    fn read(text: &'static str, format: RowFormat) -> Vec<Result<Row, RowProblem>> {
        Rows::new(Box::new(text.as_bytes()), format)
            .unwrap()
            .collect()
    }

    #[test]
    fn jsonl_rows_carry_their_line_and_blank_lines_are_skipped() {
        let rows = read(
            "\u{feff}{\"a\": 1}\n\n  \n\"text\"\n{\"b\": 2}",
            RowFormat::Jsonl,
        );

        assert_eq!(
            rows,
            [
                Ok(Row {
                    line: 1,
                    value: json!({ "a": 1 })
                }),
                Ok(Row {
                    line: 4,
                    value: json!("text")
                }),
                Ok(Row {
                    line: 5,
                    value: json!({ "b": 2 })
                }),
            ]
        );
    }

    #[test]
    fn a_malformed_jsonl_line_is_a_problem_that_does_not_quote_it() {
        let rows = read(
            "{\"a\": 1}\n{\"secret\": SENTINEL\n{\"b\": 2}\n",
            RowFormat::Jsonl,
        );

        let problem = rows[1].clone().unwrap_err();
        assert_eq!(problem.line, Some(2));
        assert!(problem.describe().starts_with("line 2: not valid JSON"));
        assert!(!problem.message.contains("SENTINEL"), "{problem:?}");
        assert!(rows[2].is_ok(), "the next line is still read");
    }

    #[test]
    fn csv_rows_are_objects_of_strings_keyed_by_the_header() {
        let mut rows = Rows::new(
            Box::new("id,body\n1,\"Refund, please\"\n2,\"multi\nline\"\n3,x\n".as_bytes()),
            RowFormat::Csv,
        )
        .unwrap();

        assert_eq!(rows.columns().unwrap(), ["id", "body"]);
        let read: Vec<_> = rows.by_ref().map(Result::unwrap).collect();
        assert_eq!(
            read[0].value,
            json!({ "id": "1", "body": "Refund, please" })
        );
        assert_eq!(
            read.iter().map(|row| row.line).collect::<Vec<_>>(),
            [2, 3, 5]
        );
    }

    #[test]
    fn a_csv_row_of_the_wrong_width_or_a_repeated_column_is_a_problem() {
        let rows = read("a,b\n1,2\n1,2,SENTINEL\n", RowFormat::Csv);
        let repeated = Rows::new(Box::new("a,b,a\n".as_bytes()), RowFormat::Csv)
            .err()
            .unwrap();

        let problem = rows[1].clone().unwrap_err();
        assert_eq!(
            problem.describe(),
            "line 3: has 3 fields where the header has 2"
        );
        assert_eq!(
            repeated.message,
            "the CSV header names the column `a` twice"
        );
    }

    #[test]
    fn the_format_follows_the_extension() {
        use std::path::Path;

        assert_eq!(RowFormat::of_path(Path::new("x.CSV")), RowFormat::Csv);
        assert_eq!(RowFormat::of_path(Path::new("x.jsonl")), RowFormat::Jsonl);
        assert_eq!(RowFormat::of_path(Path::new("x")), RowFormat::Jsonl);
    }
}
