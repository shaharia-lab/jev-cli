//! Writes the Markdown command reference, `docs/commands.md`, from the command tree `jev` parses
//! with, so that it says what `jev --help` and `jev spec` say.
//!
//! ```text
//! cargo run -p jev-cli --example command-reference -- docs/commands.md
//! ```
//!
//! `make reference` runs this; a test fails when the committed file is out of date.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(file) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: command-reference <file>");
        return ExitCode::from(2);
    };
    match std::fs::write(&file, jev_cli::command_reference()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: could not write {}: {error}", file.display());
            ExitCode::FAILURE
        }
    }
}
