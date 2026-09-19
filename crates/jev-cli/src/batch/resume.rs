//! `--resume`: what an earlier run already wrote to the output, so that it is not sent again.
//!
//! There is no state file: the output is the state. A row whose id has an `ok` record is done; a
//! row with only `error` records, or none, is sent again, and its new record is appended.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

use super::Ids;
use crate::error::CliError;

/// What an earlier run left in the output file.
#[derive(Debug, Default)]
pub(crate) struct Resumed {
    /// The ids recorded `ok`.
    pub(crate) done: Ids,
    /// Complete records in the file, whatever their status.
    pub(crate) records: u64,
    /// The length of the file up to the end of its last complete line.
    pub(crate) complete_len: u64,
    /// The bytes after that: a record cut short by a crash, which is discarded.
    pub(crate) partial_len: u64,
}

/// Reads the records of an earlier run. A file that does not exist yet has none.
///
/// # Errors
///
/// A usage error when the file cannot be read, or holds a complete line that is not a batch
/// record. The line is named by number; its content is never quoted, as it may be customer data.
pub(crate) fn read(path: &str) -> Result<Resumed, CliError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Resumed::default()),
        Err(error) => return Err(unreadable(path, &error)),
    };
    let mut reader = BufReader::new(file);
    let mut resumed = Resumed::default();
    let mut buffer = Vec::new();
    let mut line = 0_u64;
    loop {
        buffer.clear();
        let read = reader
            .read_until(b'\n', &mut buffer)
            .map_err(|error| unreadable(path, &error))?;
        if read == 0 {
            return Ok(resumed);
        }
        let length = u64::try_from(read).unwrap_or(u64::MAX);
        if buffer.last() != Some(&b'\n') {
            // Every record is written with its newline in one write, so a line without one was
            // cut short. Whatever it held is sent again.
            resumed.partial_len = length;
            return Ok(resumed);
        }
        line += 1;
        resumed.complete_len += length;
        if buffer.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let (id, ok) = record(&buffer).ok_or_else(|| {
            CliError::usage(format!(
                "line {line} of {path} is not a `jev batch run` record, so the run cannot be resumed from it"
            ))
            .hint("--resume continues an output written by `jev batch run`; check the --out path")
        })?;
        resumed.records += 1;
        if ok {
            resumed.done.add(&id);
        }
    }
}

/// The id of a record, and whether it is `ok`.
fn record(line: &[u8]) -> Option<(Value, bool)> {
    let Value::Object(mut record) = serde_json::from_slice(line).ok()? else {
        return None;
    };
    let id = record
        .remove("id")
        .filter(|id| id.is_string() || id.is_number())?;
    match record.get("status").and_then(Value::as_str)? {
        "ok" => Some((id, true)),
        "error" => Some((id, false)),
        _ => None,
    }
}

/// Cuts off a record left incomplete by a crash, so that the next one starts on a line of its own.
///
/// # Errors
///
/// A usage error when the file cannot be changed.
pub(crate) fn discard_partial(path: &str, resumed: &Resumed) -> Result<(), CliError> {
    if resumed.partial_len == 0 {
        return Ok(());
    }
    OpenOptions::new()
        .write(true)
        .open(Path::new(path))
        .and_then(|file| file.set_len(resumed.complete_len))
        .map_err(|error| {
            CliError::usage(format!(
                "cannot remove the incomplete last line of {path}: {error}"
            ))
            .hint("check that the --out file can be written")
        })
}

fn unreadable(path: &str, error: &io::Error) -> CliError {
    CliError::usage(format!(
        "cannot read the output file {path} to resume: {error}"
    ))
    .hint("check the path passed to --out")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::{discard_partial, read};

    fn file(name: &str, content: &str) -> String {
        let dir = std::env::temp_dir().join(format!("jev-resume-unit-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        path.to_str().unwrap().to_owned()
    }

    #[test]
    fn ok_ids_are_done_error_ids_are_not_and_a_cut_off_line_is_discarded() {
        let path = file(
            "cut.jsonl",
            "{\"id\": 1, \"status\": \"ok\"}\n{\"id\": \"b\", \"status\": \"error\"}\n\n{\"id\": \"7\", \"status\": \"ok\"}\n{\"id\": 4, \"sta",
        );

        let resumed = read(&path).unwrap();
        discard_partial(&path, &resumed).unwrap();

        assert!(resumed.done.contains(&json!(1)));
        assert!(resumed.done.contains(&json!(7)), "\"7\" and 7 are one id");
        assert!(!resumed.done.contains(&json!("b")));
        assert!(!resumed.done.contains(&json!(4)));
        assert_eq!((resumed.records, resumed.partial_len), (3, 14));
        assert!(fs::read_to_string(&path).unwrap().ends_with("\"ok\"}\n"));
        assert_eq!(read(&path).unwrap().partial_len, 0);
    }

    #[test]
    fn a_missing_file_has_nothing_and_a_foreign_line_is_refused_without_quoting_it() {
        let missing = read(&file("unused", "").replace("unused", "nothing-here.jsonl")).unwrap();
        let path = file(
            "foreign.jsonl",
            "{\"id\": 1, \"status\": \"ok\"}\n{\"SENTINEL\": true}\n",
        );

        let error = read(&path).unwrap_err();

        assert_eq!((missing.records, missing.done.len()), (0, 0));
        assert_eq!(error.exit.code(), 2);
        assert!(error.message.starts_with("line 2 of "), "{}", error.message);
        assert!(!error.message.contains("SENTINEL"));
    }
}
