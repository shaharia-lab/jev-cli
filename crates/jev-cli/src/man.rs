//! Man pages for `jev` and every command, rendered from the tree `jev` parses with and its help,
//! so that they say what `--help` says. Releases ship them; see `examples/dist-assets.rs`.

use std::io;
use std::path::Path;

/// Writes `jev.1`, and `jev-<command>.1` for every visible command, such as `jev-auth-login.1`,
/// into `directory`.
pub(crate) fn write_all(directory: &Path) -> io::Result<()> {
    let mut root = crate::cli::command();
    // Gives every subcommand its full `jev-...` name, which the page and its file are named after.
    root.build();
    write(root, directory)
}

fn write(command: clap::Command, directory: &Path) -> io::Result<()> {
    for sub in command.get_subcommands().filter(|sub| !sub.is_hide_set()) {
        write(sub.clone(), directory)?;
    }
    clap_mangen::Man::new(command)
        .source(format!("jev {}", env!("CARGO_PKG_VERSION")))
        .manual("jev manual")
        .generate_to(directory)
        .map(|_| ())
}
