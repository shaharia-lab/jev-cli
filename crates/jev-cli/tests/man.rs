//! The man pages `examples/dist-assets.rs` writes: one for `jev` and one for every command, from
//! the same tree and help as `--help`.

// Clippy's `allow-unwrap-in-tests` and `allow-panic-in-tests` cover `#[test]` functions only, not
// the helper functions an integration test shares, where failing fast is equally the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

/// The pages, written to a directory of their own for each test, since tests run in parallel.
fn pages(test: &str) -> PathBuf {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("man")
        .join(test);
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    jev_cli::write_man_pages(&directory).unwrap();
    directory
}

/// Every visible command below `command`, as the words after `jev`.
fn paths(command: &clap::Command, prefix: &[String], found: &mut Vec<Vec<String>>) {
    for sub in command.get_subcommands().filter(|sub| !sub.is_hide_set()) {
        let mut path = prefix.to_vec();
        path.push(sub.get_name().to_owned());
        paths(sub, &path, found);
        found.push(path);
    }
}

#[test]
fn jev_and_every_command_have_a_page_with_their_help() {
    let directory = pages("every-command");
    let tree = jev_cli::command();
    let mut commands = Vec::new();
    paths(&tree, &[], &mut commands);

    let root = std::fs::read_to_string(directory.join("jev.1")).unwrap();
    assert!(root.contains("Exit codes:"), "{root}");
    assert!(commands.len() > 20, "{commands:?}");
    for path in commands {
        let file = directory.join(format!("jev-{}.1", path.join("-")));
        let page = std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("no page for `jev {}`: {error}", path.join(" ")));
        assert!(
            page.starts_with(".ie \\n(.g .ds Aq"),
            "{}: {page}",
            file.display()
        );
        assert!(
            !page.contains('\u{1b}'),
            "{} has escape codes",
            file.display()
        );
        let version = format!("\"jev {}\" \"jev manual\"", env!("CARGO_PKG_VERSION"));
        assert!(page.contains(&version), "{}: {page}", file.display());
    }

    let noul = std::fs::read_to_string(directory.join("jev-noul.1")).unwrap();
    for section in [
        "When to use:",
        "Exit codes:",
        "Examples:",
        "\\-\\-fail\\-under",
    ] {
        assert!(noul.contains(section), "`{section}` is missing: {noul}");
    }
}

#[test]
fn hidden_commands_have_no_page() {
    let directory = pages("hidden");

    let names: Vec<String> = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.iter().all(|name| !name.starts_with("jev-debug")),
        "{names:?}"
    );
}
