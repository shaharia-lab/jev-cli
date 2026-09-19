//! Entry points for the fuzz targets in `fuzz/`, which call them with arbitrary bytes. Every input
//! must be handled without a panic (PRD REL-7), and whatever is produced is thrown away.
//!
//! Compiled only for `cargo fuzz` (which sets `--cfg fuzzing`) and for this crate's tests, which
//! run each entry point on a few inputs so that the targets keep compiling. Not an API.

use std::io::{self, Cursor};
use std::time::Duration;

use indexmap::IndexMap;
use jev_client::{ApiKey, Response};
use serde_json::{Value, json};
use toml_edit::DocumentMut;

use crate::batch::{self, Mapping, RowFormat, Rows, StateMapping};
use crate::config::{ConfigFile, Flags, Settings};
use crate::env::Env;
use crate::error::CliError;
use crate::evaluate::{self, PrepareOptions};
use crate::gate::{self, AbstainBand, Condition};
use crate::input::{self, InputFormat};
use crate::output::{Format, Output, ResultEnvelope, Ui};

/// The settings of a machine with no configuration and no environment.
pub(crate) fn default_settings() -> Result<Settings, CliError> {
    Settings::resolve(
        &Flags::default(),
        &Env::default(),
        None,
        &IndexMap::new(),
        &IndexMap::new(),
    )
}

/// A request file, read as JSON and as YAML, then prepared as `jev eval` would prepare it: with
/// its own state, and with a state given on the command line in strict mode.
pub fn request_file(data: &[u8]) {
    let (Ok(text), Ok(settings)) = (std::str::from_utf8(data), default_settings()) else {
        return;
    };
    for format in [InputFormat::Json, InputFormat::Yaml] {
        let Ok(document) = input::parse_document(text, "request", "request", Some(format)) else {
            continue;
        };
        let _ = evaluate::prepare(document.clone(), &settings, &PrepareOptions::default());
        let options = PrepareOptions {
            state: Some(json!("a state from the command line")),
            strict: true,
            skip_size_check: false,
        };
        let _ = evaluate::prepare(document, &settings, &options);
    }
}

/// A response as `jev eval` shows it. The input is three lines: a `--field` path (empty for
/// none), an `--assert` condition or an `--abstain-band`, and the response body. The body is
/// rendered in every output format, gated, and has the path picked out of it.
pub fn output(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let mut lines = text.splitn(3, '\n');
    let (Some(field), Some(gate_text), Some(body)) = (lines.next(), lines.next(), lines.next())
    else {
        return;
    };
    let Ok(response) = serde_json::from_str::<Response>(body) else {
        return;
    };
    let raw = serde_json::from_str::<Value>(body).ok();
    let mut envelope = ResultEnvelope::new(
        &response,
        raw.as_ref(),
        "jev-latest",
        Some("req_fuzz".to_owned()),
        Duration::from_millis(1),
        None,
    );
    let conditions: Vec<Condition> = Condition::parse(gate_text).into_iter().collect();
    let first_id = envelope.answers.keys().next().cloned().unwrap_or_default();
    let abstain = AbstainBand::parse(gate_text)
        .ok()
        .map(|band| (first_id.as_str(), band));
    if let Ok(outcome) = gate::decide(&conditions, abstain, &envelope.answers) {
        envelope.gate = outcome;
    }
    let field = (!field.is_empty()).then(|| field.to_owned());
    for format in [Format::Table, Format::Json, Format::Yaml, Format::Jsonl] {
        for ui in [Ui::new(false, false), Ui::new(true, true)] {
            let output = Output {
                format,
                field: None,
                ui,
            };
            let _ = output.emit(&envelope, &mut io::sink());
        }
    }
    let output = Output {
        format: Format::Json,
        field,
        ui: Ui::new(false, false),
    };
    let _ = output.emit(&envelope, &mut io::sink());
}

/// A batch input. The first byte picks the format, how a row becomes a state and whether rows
/// have an id field; the rest is the input, which is checked as `jev batch run` checks a file
/// before anything is sent.
pub fn batch_rows(data: &[u8]) {
    let Some((&first, input)) = data.split_first() else {
        return;
    };
    let format = if first & 1 == 0 {
        RowFormat::Jsonl
    } else {
        RowFormat::Csv
    };
    let mapping = Mapping {
        state: match (first >> 1) % 3 {
            0 => StateMapping::WholeRow,
            1 => StateMapping::Field("text".to_owned()),
            _ => StateMapping::Fields(vec!["id".to_owned(), "text".to_owned()]),
        },
        id_field: (first & 8 != 0).then(|| "id".to_owned()),
    };
    let Ok(rows) = Rows::new(Box::new(Cursor::new(input.to_vec())), format) else {
        return;
    };
    if let Some(columns) = rows.columns() {
        let _ = mapping.check_columns(columns);
    }
    let _ = batch::preflight(rows, &mapping);
}

/// A `config.toml` and a `credentials` file, each read as `jev` reads them at start-up. A
/// configuration that reads is also resolved into settings, given a new profile and written back,
/// and what is written must read again.
pub fn config(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(mut file) = ConfigFile::parse(text, "config.toml") {
        let _ = Settings::resolve(
            &Flags::default(),
            &Env::default(),
            file.active_profile.as_deref(),
            &file.profiles,
            &file.shared,
        );
        file.create_profile("fuzz");
        file.set_active_profile("fuzz");
        let written = file.to_toml();
        assert!(
            ConfigFile::parse(&written, "config.toml").is_ok(),
            "jev wrote a configuration it cannot read"
        );
    }
    if let Ok(document) = text.parse::<DocumentMut>()
        && let Some(key) = crate::credentials::stored_key(&document, "default")
    {
        let _ = ApiKey::new(key.to_owned());
    }
}

/// Lines from an MCP client, answered by a server that has no API key.
pub fn mcp(data: &[u8]) {
    crate::commands::mcp::answer_offline(data);
}

#[cfg(test)]
mod tests {
    use super::{batch_rows, config, mcp, output, request_file};

    const REQUEST: &str =
        r#"{"state": "s", "questions": {"q": {"type": "noul", "instructions": "Is it?"}}}"#;
    const RESPONSE: &str = r#"{"model": "jev-1.13.0", "answers": {"q": {"type": "noul", "noul": 0.7}, "c": {"type": "choice", "choice": "a", "probabilities": {"a": 1.0}}}, "usage": {"input_tokens": 9}}"#;

    #[test]
    fn the_request_file_entry_point_takes_anything() {
        for input in [
            REQUEST,
            "state: s\nquestions:\n  q: {type: score, criteria: [a]}\n",
            "- [",
            "",
        ] {
            request_file(input.as_bytes());
        }
    }

    #[test]
    fn the_output_entry_point_takes_anything() {
        for input in [
            format!("answers.q.noul\nq >= 0.5\n{RESPONSE}"),
            format!("answers[9]\n0.4,0.6\n{RESPONSE}"),
            format!("\nc in a,b\n{RESPONSE}"),
            "\n\n{}".to_owned(),
        ] {
            output(input.as_bytes());
        }
    }

    #[test]
    fn the_batch_entry_point_takes_anything() {
        let jsonl = b"{\"id\": 1, \"text\": \"a\"}\n{\"id\": 1}\nnot json\n";
        let csv = b"id,text\n1,a\n1,b,c\n";
        for first in 0..16_u8 {
            for input in [&jsonl[..], &csv[..], b"", b"\xff\xfe"] {
                let mut data = vec![first];
                data.extend_from_slice(input);
                batch_rows(&data);
            }
        }
    }

    #[test]
    fn the_config_entry_point_takes_anything() {
        for input in [
            "active_profile = \"work\"\n[profiles.work]\ntimeout = \"10s\"\n",
            "[profiles.default]\napi_key = \"sk-test\"\n",
            "profiles = 1",
            "[",
        ] {
            config(input.as_bytes());
        }
    }

    #[test]
    fn the_mcp_entry_point_takes_anything() {
        let session = concat!(
            r#"{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}"#,
            "\n",
            r#"{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "validate", "arguments": {"request": {}}}}"#,
            "\n",
            r#"{"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "noul", "arguments": {"state": "s", "instructions": "Is it?"}}}"#,
            "\nnot json\n[]\n"
        );
        mcp(session.as_bytes());
    }
}
