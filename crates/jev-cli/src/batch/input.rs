//! Reading the rows of a batch, one at a time, from a JSONL or CSV stream, or from a file holding
//! one JSON array.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

use clap::ValueEnum;
use schemars::JsonSchema;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::CliError;

/// The largest JSON array input, in bytes. JSONL and CSV stream, but an array is read whole, so
/// this bounds the memory it takes.
pub(crate) const MAX_JSON_INPUT_BYTES: u64 = 50 * 1024 * 1024;

/// The UTF-8 byte order mark, which some editors write at the start of a file.
const BOM: &[u8] = b"\xEF\xBB\xBF";

/// The format of a batch's input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RowFormat {
    /// One JSON value per line
    Jsonl,
    /// Comma-separated values with a header row
    Csv,
    /// One JSON array whose elements are the rows (a file of at most 50 MB)
    Json,
}

impl RowFormat {
    /// The format of a file: `.csv` is CSV, a `.json` file holding one JSON array is JSON, and
    /// anything else is JSONL.
    ///
    /// `open` is called only for a `.json` file. One that cannot be opened is taken for JSONL, and
    /// the error is reported when it is opened to be read.
    pub(crate) fn of_path<R: Read>(path: &Path, open: impl FnOnce() -> Option<R>) -> Self {
        match path.extension().and_then(|extension| extension.to_str()) {
            Some(extension) if extension.eq_ignore_ascii_case("csv") => Self::Csv,
            Some(extension)
                if extension.eq_ignore_ascii_case("json") && open().is_some_and(holds_an_array) =>
            {
                Self::Json
            }
            _ => Self::Jsonl,
        }
    }
}

/// Whether an input is meant as one JSON array: it starts with `[` and is one JSON value and nothing
/// else, or ends before that value does. It is parsed without being kept, so the check takes no
/// memory, and only up to [`MAX_JSON_INPUT_BYTES`]: an array cut short there, or by the end of the
/// file, is still taken for one, so that reading it says what is wrong. A JSONL file whose lines
/// are arrays has more after its first value, so it stays JSONL.
fn holds_an_array(reader: impl Read) -> bool {
    let mut reader = BufReader::new(reader.take(MAX_JSON_INPUT_BYTES + 1));
    let mut start = true;
    loop {
        let Ok(buffer) = reader.fill_buf() else {
            return false;
        };
        let mut bytes = buffer;
        if start {
            bytes = bytes.strip_prefix(BOM).unwrap_or(bytes);
        }
        let skipped = buffer.len() - bytes.len();
        match bytes.iter().position(|byte| !byte.is_ascii_whitespace()) {
            Some(position) if bytes.get(position) == Some(&b'[') => {
                reader.consume(skipped + position);
                break;
            }
            Some(_) => return false,
            None if buffer.is_empty() => return false,
            None => {
                let length = buffer.len();
                reader.consume(length);
                start = false;
            }
        }
    }
    let mut parser = serde_json::Deserializer::from_reader(reader);
    match IgnoredAny::deserialize(&mut parser).and_then(|_| parser.end()) {
        Ok(()) => true,
        Err(error) => error.is_eof(),
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
    /// The 1-based line the row starts on; for a JSON array, the element's 1-based index.
    pub(crate) line: u64,
    /// The row: any JSON value for JSONL and JSON, an object of strings for CSV.
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

/// The rows of an input, read lazily: only the row being handed out is in memory, except for a
/// JSON array, which is read whole.
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
    Json {
        items: std::vec::IntoIter<Value>,
        index: u64,
    },
}

impl Rows {
    /// Starts reading. A CSV header is read at once, so that a missing column is known before
    /// any row is.
    ///
    /// # Errors
    ///
    /// A usage error when a CSV header is missing, unreadable or names a column twice, and when
    /// a JSON input is over [`MAX_JSON_INPUT_BYTES`], unreadable, or not one array.
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
            RowFormat::Json => Ok(Self {
                inner: Inner::Json {
                    items: read_array(reader)?.into_iter(),
                    index: 0,
                },
                columns: None,
                finished: false,
            }),
        }
    }

    /// The columns of a CSV input; `None` for JSONL and JSON, whose rows each have their own
    /// fields.
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
            Inner::Json { items, index } => items.next().map(|value| {
                *index += 1;
                Ok(Row {
                    line: *index,
                    value,
                })
            }),
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

/// Reads a JSON input whole, as the array of its rows.
fn read_array(reader: Box<dyn Read + Send>) -> Result<Vec<Value>, CliError> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_JSON_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| CliError::usage(format!("the input cannot be read: {error}")))?;
    if bytes.len() as u64 > MAX_JSON_INPUT_BYTES {
        return Err(CliError::usage(format!(
            "the input is over the {} MB limit for a JSON array; nothing was sent",
            MAX_JSON_INPUT_BYTES / 1024 / 1024
        ))
        .hint("convert it to JSONL, which streams with no limit: jq -c '.[]' items.json > items.jsonl"));
    }
    let bytes = bytes.strip_prefix(BOM).unwrap_or(&bytes);
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Array(items)) => Ok(items),
        Ok(_) => Err(CliError::usage(
            "the input is not a JSON array of rows; nothing was sent",
        )
        .hint("pass a file holding `[...]`, or read one JSON value per line with --input-format jsonl")),
        // serde_json reports a position and a category, never the text itself.
        Err(error) => Err(CliError::usage(format!(
            "the input is not valid JSON ({error}); nothing was sent"
        ))
        .hint("check the file, e.g. with `jq . <file>`")),
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

        let never = || -> Option<&[u8]> { panic!("only a `.json` file is looked into") };
        assert_eq!(
            RowFormat::of_path(Path::new("x.CSV"), never),
            RowFormat::Csv
        );
        assert_eq!(
            RowFormat::of_path(Path::new("x.jsonl"), never),
            RowFormat::Jsonl
        );
        assert_eq!(RowFormat::of_path(Path::new("x"), never), RowFormat::Jsonl);
    }

    #[test]
    fn a_json_file_is_an_array_only_when_it_holds_one_array_and_nothing_else() {
        use std::path::Path;

        let of =
            |text: &'static str| RowFormat::of_path(Path::new("x.JSON"), || Some(text.as_bytes()));
        assert_eq!(
            of("\u{feff} \n [{\"a\": 1},\n {\"a\": 2}]\n"),
            RowFormat::Json
        );
        assert_eq!(of("[]"), RowFormat::Json);
        // JSONL, as a `.json` file has always been read.
        assert_eq!(of("{\"a\": 1}\n{\"a\": 2}\n"), RowFormat::Jsonl);
        assert_eq!(of("[1, 2]\n[3, 4]\n"), RowFormat::Jsonl);
        assert_eq!(of("[1, 2\n[3, 4]\n"), RowFormat::Jsonl);
        assert_eq!(of(""), RowFormat::Jsonl);
        // Cut short, it is an array, so that reading it reports the end or the limit.
        assert_eq!(of("[1, 2"), RowFormat::Json);
        let over = std::io::Read::chain(
            &b"["[..],
            std::io::Read::take(std::io::repeat(b' '), super::MAX_JSON_INPUT_BYTES),
        );
        assert_eq!(
            RowFormat::of_path(Path::new("x.json"), || Some(over)),
            RowFormat::Json
        );
        assert_eq!(
            RowFormat::of_path(Path::new("x.json"), || None::<&[u8]>),
            RowFormat::Jsonl
        );
    }

    #[test]
    fn json_rows_are_the_elements_of_the_array_numbered_from_one() {
        let rows = read(
            "\u{feff}[\n  {\"a\": 1},\n  \"text\",\n  [2]\n]",
            RowFormat::Json,
        );

        assert_eq!(
            rows,
            [
                Ok(Row {
                    line: 1,
                    value: json!({ "a": 1 })
                }),
                Ok(Row {
                    line: 2,
                    value: json!("text")
                }),
                Ok(Row {
                    line: 3,
                    value: json!([2])
                }),
            ]
        );
        assert!(read("[]", RowFormat::Json).is_empty());
    }

    #[test]
    fn a_json_input_that_is_not_one_array_is_refused_without_quoting_it() {
        let error = |text: &'static str| {
            Rows::new(Box::new(text.as_bytes()), RowFormat::Json)
                .err()
                .unwrap()
        };

        let object = error("{\"secret\": \"SENTINEL\"}");
        assert!(
            object.message.starts_with("the input is not a JSON array"),
            "{object:?}"
        );
        for malformed in [
            error("[{\"secret\": SENTINEL}]"),
            error("[1] [2]"),
            error(""),
        ] {
            assert!(
                malformed.message.starts_with("the input is not valid JSON"),
                "{malformed:?}"
            );
            assert!(!format!("{malformed:?}").contains("SENTINEL"));
        }
        assert!(!format!("{object:?}").contains("SENTINEL"));
    }

    #[test]
    fn a_json_input_over_the_limit_is_refused_before_it_is_parsed() {
        let limit = usize::try_from(super::MAX_JSON_INPUT_BYTES).unwrap();
        let exactly = format!("[{}]", " ".repeat(limit - 2));
        let over = format!("[{}]", " ".repeat(limit - 1));

        assert!(Rows::new(Box::new(std::io::Cursor::new(exactly)), RowFormat::Json).is_ok());
        let error = Rows::new(Box::new(std::io::Cursor::new(over)), RowFormat::Json)
            .err()
            .unwrap();
        assert!(error.message.contains("50 MB"), "{error:?}");
        assert!(format!("{error:?}").contains("jq -c '.[]'"));
    }
}
