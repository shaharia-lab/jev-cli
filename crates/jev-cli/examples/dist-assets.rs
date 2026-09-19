//! Writes the files a release ships next to the `jev` binary: a man page for `jev` and for every
//! command, and the completion script of every shell. Both come from the command tree `jev` parses
//! with, so they say what `jev --help` says.
//!
//! ```text
//! cargo run -p jev-cli --example dist-assets --locked -- <directory>
//! ```
//!
//! The directory gets `man/man1/*.1` and `completions/` (`jev.bash`, `_jev`, `jev.fish`,
//! `_jev.ps1`). It runs on the build host, so it also serves cross-compiled targets.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::ValueEnum;
use jev_cli::Shell;

fn main() -> ExitCode {
    let Some(directory) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: dist-assets <directory>");
        return ExitCode::from(2);
    };
    match write(&directory) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: could not write to {}: {error}", directory.display());
            ExitCode::FAILURE
        }
    }
}

fn write(directory: &std::path::Path) -> std::io::Result<()> {
    let man = directory.join("man").join("man1");
    std::fs::create_dir_all(&man)?;
    jev_cli::write_man_pages(&man)?;

    let completions = directory.join("completions");
    std::fs::create_dir_all(&completions)?;
    for shell in Shell::value_variants() {
        std::fs::write(
            completions.join(shell.file_name()),
            jev_cli::completion_script(*shell),
        )?;
    }
    Ok(())
}
