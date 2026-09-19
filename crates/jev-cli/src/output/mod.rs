//! The output layer: one place decides the format, and every command renders through it.
//!
//! The rule is simple and has no exceptions: **stdout is data**. A person at a terminal gets a
//! readable form; a pipe gets JSON, without being asked. Notices, warnings and errors go to stderr.

mod envelope;
mod field;
mod table;
mod ui;

use std::io::{self, Write};

use clap::ValueEnum;
use serde::Serialize;
use serde_json::Value;

// First used outside the test hooks by `jev eval` (issue #9).
#[cfg_attr(not(feature = "internal-test-hooks"), allow(unused_imports))]
pub(crate) use envelope::ResultEnvelope;
pub(crate) use table::{Cell, Table};
pub(crate) use ui::Ui;

use crate::error::CliError;

/// An output format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Format {
    /// Readable text for a person at a terminal.
    Table,
    /// One pretty-printed JSON document. The default when stdout is not a terminal.
    Json,
    /// One YAML document.
    Yaml,
    /// Compact JSON, one record per line.
    Jsonl,
}

impl Format {
    /// The name used on the command line and in `config.toml`.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Jsonl => "jsonl",
        }
    }

    /// The format to use: what was asked for, and otherwise readable text on a terminal and JSON
    /// everywhere else.
    pub(crate) fn resolve(requested: Option<Self>, stdout_is_terminal: bool) -> Self {
        requested.unwrap_or(if stdout_is_terminal {
            Self::Table
        } else {
            Self::Json
        })
    }

    /// Whether a program, not a person, is expected to read the output. Errors follow suit.
    pub(crate) const fn is_machine_readable(self) -> bool {
        !matches!(self, Self::Table)
    }

    /// Finds `-o`/`--output` in raw arguments, for when they could not be parsed and the error
    /// still has to be printed in the right format.
    pub(crate) fn scan(arguments: &[String]) -> Option<Self> {
        let mut arguments = arguments.iter();
        while let Some(argument) = arguments.next() {
            let value = match argument.as_str() {
                "--" => return None,
                "-o" | "--output" => arguments.next().map(String::as_str),
                other => other.strip_prefix("--output=").or_else(|| {
                    other
                        .strip_prefix("-o")
                        .filter(|rest| !rest.is_empty() && !other.starts_with("--"))
                }),
            };
            if let Some(format) = value.and_then(|value| Self::from_str(value, true).ok()) {
                return Some(format);
            }
        }
        None
    }
}

/// Something a command can print.
///
/// Machine formats come from `Serialize`; only the human form is written by hand.
pub(crate) trait Render: Serialize {
    /// The readable form, ending in a newline.
    fn human(&self, ui: Ui) -> String;

    /// The records for `jsonl`, one per line. A single result is a single record; a list overrides
    /// this to give one record per item.
    fn records(&self) -> Option<Vec<Value>> {
        None
    }
}

/// Where a command's result goes, and how.
#[derive(Debug)]
pub(crate) struct Output {
    pub(crate) format: Format,
    pub(crate) field: Option<String>,
    pub(crate) ui: Ui,
}

impl Output {
    /// Writes `item` to `stdout` in the chosen format, or just one field of it.
    ///
    /// # Errors
    ///
    /// A usage error when `--field` does not match. A closed pipe is not an error: the reader has
    /// what it wanted.
    pub(crate) fn emit(&self, item: &impl Render, stdout: &mut dyn Write) -> Result<(), CliError> {
        let text = self.render(item)?;
        match stdout
            .write_all(text.as_bytes())
            .and_then(|()| stdout.flush())
        {
            Err(error) if error.kind() != io::ErrorKind::BrokenPipe => Err(CliError::internal(
                format!("could not write to stdout: {error}"),
            )),
            _ => Ok(()),
        }
    }

    fn render(&self, item: &impl Render) -> Result<String, CliError> {
        let serialise = |error: &dyn std::fmt::Display| {
            CliError::internal(format!("could not serialise the result: {error}"))
        };

        if let Some(path) = &self.field {
            let value = serde_json::to_value(item).map_err(|error| serialise(&error))?;
            return Ok(format!("{}\n", field::to_raw(field::select(&value, path)?)));
        }
        match self.format {
            Format::Table => Ok(item.human(self.ui)),
            Format::Json => serde_json::to_string_pretty(item)
                .map(|json| format!("{json}\n"))
                .map_err(|error| serialise(&error)),
            Format::Yaml => serde_saphyr::to_string(item).map_err(|error| serialise(&error)),
            Format::Jsonl => {
                let records = match item.records() {
                    Some(records) => records,
                    None => vec![serde_json::to_value(item).map_err(|error| serialise(&error))?],
                };
                Ok(records.iter().fold(String::new(), |mut lines, record| {
                    lines.push_str(&record.to_string());
                    lines.push('\n');
                    lines
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};

    use serde::Serialize;
    use serde_json::{Value, json};

    use super::{Format, Output, Render, Ui};

    #[derive(Serialize)]
    struct Models {
        models: Vec<&'static str>,
        note: Option<&'static str>,
    }

    impl Render for Models {
        fn human(&self, _: Ui) -> String {
            format!("{}\n", self.models.join("\n"))
        }

        fn records(&self) -> Option<Vec<Value>> {
            Some(
                self.models
                    .iter()
                    .map(|name| json!({ "name": name }))
                    .collect(),
            )
        }
    }

    fn emit(format: Format, field: Option<&str>) -> Result<String, crate::error::CliError> {
        let output = Output {
            format,
            field: field.map(str::to_owned),
            ui: Ui::plain(),
        };
        let mut stdout = Vec::new();
        output.emit(
            &Models {
                models: vec!["jev-latest", "jev-preview"],
                note: None,
            },
            &mut stdout,
        )?;
        Ok(String::from_utf8(stdout).unwrap())
    }

    #[test]
    fn a_terminal_gets_text_and_a_pipe_gets_json_unless_told_otherwise() {
        assert_eq!(Format::resolve(None, true), Format::Table);
        assert_eq!(Format::resolve(None, false), Format::Json);
        assert_eq!(Format::resolve(Some(Format::Yaml), true), Format::Yaml);
        assert_eq!(Format::resolve(Some(Format::Table), false), Format::Table);
        assert!(!Format::Table.is_machine_readable());
        assert!(
            Format::Json.is_machine_readable()
                && Format::Yaml.is_machine_readable()
                && Format::Jsonl.is_machine_readable()
        );
    }

    #[test]
    fn every_format_renders_the_same_data() {
        assert_eq!(
            emit(Format::Table, None).unwrap(),
            "jev-latest\njev-preview\n"
        );
        assert_eq!(
            emit(Format::Json, None).unwrap(),
            "{\n  \"models\": [\n    \"jev-latest\",\n    \"jev-preview\"\n  ],\n  \"note\": null\n}\n"
        );
        assert_eq!(
            emit(Format::Yaml, None).unwrap(),
            "models:\n- jev-latest\n- jev-preview\nnote: null\n"
        );
        assert_eq!(
            emit(Format::Jsonl, None).unwrap(),
            "{\"name\":\"jev-latest\"}\n{\"name\":\"jev-preview\"}\n"
        );
    }

    #[test]
    fn a_field_wins_over_the_format_and_prints_raw() {
        for format in [Format::Table, Format::Json, Format::Yaml, Format::Jsonl] {
            assert_eq!(emit(format, Some("models[1]")).unwrap(), "jev-preview\n");
        }
        assert_eq!(emit(Format::Json, Some("nope")).unwrap_err().exit.code(), 2);
    }

    #[test]
    fn a_closed_pipe_is_not_an_error_but_a_failing_disk_is() {
        struct Failing(io::ErrorKind);
        impl Write for Failing {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::from(self.0))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let output = Output {
            format: Format::Json,
            field: None,
            ui: Ui::plain(),
        };
        let item = Models {
            models: vec![],
            note: None,
        };

        assert!(
            output
                .emit(&item, &mut Failing(io::ErrorKind::BrokenPipe))
                .is_ok()
        );
        assert_eq!(
            output
                .emit(&item, &mut Failing(io::ErrorKind::StorageFull))
                .unwrap_err()
                .exit
                .code(),
            1
        );
    }

    #[test]
    fn the_format_is_found_in_arguments_that_could_not_be_parsed() {
        let scan = |arguments: &[&str]| {
            Format::scan(&arguments.iter().map(|&a| a.to_owned()).collect::<Vec<_>>())
        };

        assert_eq!(scan(&["jev", "evl", "-o", "json"]), Some(Format::Json));
        assert_eq!(scan(&["jev", "--output=yaml", "evl"]), Some(Format::Yaml));
        assert_eq!(scan(&["jev", "-ojsonl"]), Some(Format::Jsonl));
        assert_eq!(scan(&["jev", "--output", "JSON"]), Some(Format::Json));
        assert_eq!(scan(&["jev", "eval"]), None);
        assert_eq!(scan(&["jev", "-o", "xml"]), None);
        assert_eq!(scan(&["jev", "--", "-o", "json"]), None);
    }
}
