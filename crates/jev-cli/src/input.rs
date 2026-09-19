//! Reading what the user supplies: the request file and the state.

use std::fs;
use std::io::Read;
use std::path::Path;

use clap::ValueEnum;
use jev_client::validate::Document;
use serde_json::Value;

use crate::error::CliError;

/// The format of a request file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum InputFormat {
    Json,
    Yaml,
}

/// How a state given outside the request file is to be read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub(crate) enum StateFormat {
    /// JSON for a `.json` file, text for everything else
    #[default]
    Auto,
    /// Send the content as one string
    Text,
    /// Parse the content as JSON and send the object or array
    Json,
}

/// Where a state given outside the request file comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StateSource {
    /// `--state <text>`
    Inline(String),
    /// `--state-file <path>`
    File(String),
    /// Standard input: `--state-file -`, or a pipe when nothing else supplies a state
    Stdin,
}

/// Reads a request file. `source` is a path, or `-` for standard input.
///
/// The format comes from `format` when given, then from the extension, and for standard input
/// from the first character: JSON starts with `{`.
///
/// # Errors
///
/// A usage error when the file cannot be read or is not well-formed. A parse error gives the
/// position and the problem, never an excerpt: a request file holds `state`.
pub(crate) fn read_document(
    source: &str,
    format: Option<InputFormat>,
    stdin: &mut dyn Read,
) -> Result<Document, CliError> {
    let (text, shown) = if source == "-" {
        (read_stdin(stdin, "the request file")?, "stdin".to_owned())
    } else {
        let text = fs::read_to_string(source).map_err(|error| {
            CliError::usage(format!("cannot read the request file {source}: {error}"))
                .hint("check the path passed to -f")
        })?;
        (text, source.to_owned())
    };

    let format = format.unwrap_or_else(|| {
        match Path::new(source)
            .extension()
            .and_then(|extension| extension.to_str())
        {
            Some(extension) if extension.eq_ignore_ascii_case("json") => InputFormat::Json,
            Some(extension)
                if extension.eq_ignore_ascii_case("yaml")
                    || extension.eq_ignore_ascii_case("yml") =>
            {
                InputFormat::Yaml
            }
            _ if text.trim_start().starts_with(['{', '[']) => InputFormat::Json,
            _ => InputFormat::Yaml,
        }
    });
    let malformed = |kind: &str, problem: String| {
        CliError::usage(format!("{shown} is not valid {kind}: {problem}")).hint(
            "fix the file; --input-format json|yaml overrides the format guessed from its name",
        )
    };
    match format {
        InputFormat::Json => {
            Document::from_json_str(&text).map_err(|error| malformed("JSON", error.to_string()))
        }
        InputFormat::Yaml => serde_saphyr::from_str::<Document>(&text)
            .map_err(|error| malformed("YAML", first_line(&error.to_string()))),
    }
}

/// Reads a state given outside the request file, in its final form: a string, or parsed JSON.
///
/// # Errors
///
/// A usage error when the source cannot be read, is empty, or is not the JSON it was said to be.
pub(crate) fn read_state(
    source: &StateSource,
    format: StateFormat,
    stdin: &mut dyn Read,
) -> Result<Value, CliError> {
    let (text, is_json_file, shown) = match source {
        StateSource::Inline(text) => (text.clone(), false, "--state".to_owned()),
        StateSource::Stdin => (
            read_stdin(stdin, "the state")?,
            false,
            "the state on stdin".to_owned(),
        ),
        StateSource::File(path) => {
            let text = fs::read_to_string(path).map_err(|error| {
                CliError::usage(format!("cannot read the state file {path}: {error}"))
                    .hint("check the path passed to --state-file")
            })?;
            let is_json = Path::new(path)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
            (text, is_json, path.clone())
        }
    };
    if text.trim().is_empty() {
        return Err(CliError::usage(format!("{shown} is empty"))
            .hint("give the content to evaluate: text, or JSON with --state-format json"));
    }

    let as_json = match format {
        StateFormat::Json => true,
        StateFormat::Text => false,
        StateFormat::Auto => is_json_file,
    };
    if !as_json {
        return Ok(Value::String(text));
    }
    serde_json::from_str(&text).map_err(|error| {
        CliError::usage(format!("{shown} is not valid JSON: {error}"))
            .hint("use --state-format text to send it as plain text")
    })
}

fn read_stdin(stdin: &mut dyn Read, what: &str) -> Result<String, CliError> {
    let mut text = String::new();
    stdin
        .read_to_string(&mut text)
        .map(|_| text)
        .map_err(|error| {
            CliError::usage(format!("cannot read {what} from stdin: {error}"))
                .hint("stdin must be UTF-8 text")
        })
}

fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("syntax error");
    line.trim_start_matches("error: ").to_owned()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use serde_json::json;

    use super::{InputFormat, StateFormat, StateSource, read_document, read_state};

    fn scratch(name: &str, content: &str) -> String {
        let path: PathBuf =
            std::env::temp_dir().join(format!("jev-input-{}-{name}", std::process::id()));
        fs::write(&path, content).unwrap();
        path.to_str().unwrap().to_owned()
    }

    const YAML: &str =
        "state: hello\nquestions:\n  q:\n    type: noul\n    instructions: Is it urgent?\n";
    const JSON: &str = r#"{"state": "hello", "questions": {"q": {"type": "noul", "instructions": "Is it urgent?"}}}"#;

    #[test]
    fn json_and_yaml_request_files_give_the_same_document() {
        let yaml = read_document(&scratch("a.yaml", YAML), None, &mut std::io::empty()).unwrap();
        let upper = read_document(&scratch("a.YML", YAML), None, &mut std::io::empty()).unwrap();
        let json = read_document(&scratch("a.json", JSON), None, &mut std::io::empty()).unwrap();

        assert_eq!(yaml.value(), json.value());
        assert_eq!(
            upper.value(),
            json.value(),
            "the extension is matched without regard to case"
        );
        assert_eq!(
            serde_json::to_string(yaml.value()).unwrap(),
            serde_json::to_string(json.value()).unwrap()
        );
    }

    #[test]
    fn stdin_is_sniffed_and_the_format_can_be_forced() {
        let sniffed_json = read_document("-", None, &mut JSON.as_bytes()).unwrap();
        let sniffed_yaml = read_document("-", None, &mut YAML.as_bytes()).unwrap();
        let forced = read_document(
            &scratch("noext", JSON),
            Some(InputFormat::Yaml),
            &mut std::io::empty(),
        )
        .unwrap();

        assert_eq!(sniffed_json.value(), sniffed_yaml.value());
        assert_eq!(forced.value(), sniffed_json.value(), "JSON is also YAML");
    }

    #[test]
    fn a_malformed_file_is_a_usage_error_that_never_quotes_the_file() {
        let secret = "SENTINEL-STATE";
        let json = read_document(
            &scratch("bad.json", &format!("{{\"state\": \"{secret}\", ")),
            None,
            &mut std::io::empty(),
        )
        .unwrap_err();
        let yaml = read_document(
            &scratch("bad.yaml", &format!("state: {secret}\n  bad indent: [\n")),
            None,
            &mut std::io::empty(),
        )
        .unwrap_err();
        let missing =
            read_document("/nonexistent/request.yaml", None, &mut std::io::empty()).unwrap_err();

        for error in [&json, &yaml, &missing] {
            assert_eq!(error.exit.code(), 2);
            assert!(!format!("{error:?}").contains(secret), "{error:?}");
        }
        assert!(
            json.message.contains("is not valid JSON"),
            "{}",
            json.message
        );
        assert!(
            yaml.message.contains("is not valid YAML"),
            "{}",
            yaml.message
        );
        assert!(
            missing.message.starts_with("cannot read the request file"),
            "{}",
            missing.message
        );
    }

    #[test]
    fn a_state_is_text_unless_it_is_said_or_named_to_be_json() {
        let object = r#"{"ticket": {"plan": "pro"}}"#;
        let inline = |format| {
            read_state(
                &StateSource::Inline(object.to_owned()),
                format,
                &mut std::io::empty(),
            )
            .unwrap()
        };
        let file = |name: &str, format| {
            read_state(
                &StateSource::File(scratch(name, object)),
                format,
                &mut std::io::empty(),
            )
            .unwrap()
        };

        assert_eq!(
            inline(StateFormat::Auto),
            json!(object),
            "a flag value is text by default"
        );
        assert_eq!(
            inline(StateFormat::Json),
            json!({ "ticket": { "plan": "pro" } })
        );
        assert_eq!(
            file("state.json", StateFormat::Auto),
            json!({ "ticket": { "plan": "pro" } })
        );
        assert_eq!(file("state.txt", StateFormat::Auto), json!(object));
        assert_eq!(file("state2.json", StateFormat::Text), json!(object));
        let piped = read_state(
            &StateSource::Stdin,
            StateFormat::Auto,
            &mut "from a pipe\n".as_bytes(),
        )
        .unwrap();
        assert_eq!(piped, json!("from a pipe\n"), "piped text is sent as it is");
    }

    #[test]
    fn an_empty_or_unparsable_state_is_a_usage_error() {
        let empty = read_state(
            &StateSource::Stdin,
            StateFormat::Auto,
            &mut "  \n".as_bytes(),
        )
        .unwrap_err();
        let not_json = read_state(
            &StateSource::Inline("plain".into()),
            StateFormat::Json,
            &mut std::io::empty(),
        )
        .unwrap_err();

        assert_eq!(empty.message, "the state on stdin is empty");
        assert!(
            not_json.message.starts_with("--state is not valid JSON"),
            "{}",
            not_json.message
        );
        assert!(not_json.hint.unwrap().contains("--state-format text"));
    }
}
