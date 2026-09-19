//! `jev`: an unofficial command-line tool for TypeSafe AI's Jev model.
#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    jev_cli::main()
}
