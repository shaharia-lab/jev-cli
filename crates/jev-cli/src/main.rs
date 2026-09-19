//! `jev`: an unofficial command-line tool for TypeSafe AI's Jev model, built for humans, shell
//! scripts and AI agents alike.
//!
//! The command surface is not implemented yet. Until it is, the binary only reports its version,
//! which is enough for packaging and continuous integration to exercise a real executable.
#![forbid(unsafe_code)]

use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    // A closed pipe (`jev | head -0`) is not an error worth a panic or a message.
    match writeln!(io::stdout(), "jev {}", jev_client::VERSION) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
