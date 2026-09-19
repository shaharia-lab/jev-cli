//! `jev` attached to a real pseudo-terminal: the default format, colour and Unicode.
//!
//! Unix only. A pseudo-terminal is the one way to see what a person sees; on a pipe `jev`
//! deliberately behaves differently.
#![cfg(unix)]
// Clippy's `allow-unwrap-in-tests` covers `#[test]` functions only, not the helpers they share.
#![allow(clippy::unwrap_used)]

#[path = "support/pty.rs"]
mod pty;

use std::process::Stdio;

/// Never the real user configuration: a directory that does not exist and that nothing writes to.
const NO_CONFIG: &str = concat!(env!("CARGO_TARGET_TMPDIR"), "/no-config");

const ISOLATED: [&str; 8] = [
    "CI",
    "NO_COLOR",
    "TERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "JEV_OUTPUT",
    "JEV_NO_INPUT",
];

/// Runs `jev` with stdout on a pseudo-terminal and returns what a person would see.
fn on_a_terminal(arguments: &[&str], environment: &[(&str, &str)]) -> String {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_jev"));
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    for variable in ISOLATED {
        command.env_remove(variable);
    }
    command.envs(environment.iter().copied());
    command.env("JEV_CONFIG_DIR", NO_CONFIG);

    let (status, seen) = pty::run(command, pty::Stream::Stdout);
    assert!(status.success(), "{status}: {seen:?}");
    // A terminal turns "\n" into "\r\n".
    seen.replace("\r\n", "\n")
}

fn piped(arguments: &[&str]) -> String {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_jev"));
    for variable in ISOLATED {
        command.env_remove(variable);
    }
    let output = command
        .env("JEV_CONFIG_DIR", NO_CONFIG)
        .args(arguments)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn the_same_result_is_text_on_a_terminal_and_json_on_a_pipe_without_any_flag() {
    let terminal = on_a_terminal(&["version"], &[("TERM", "xterm-256color")]);
    let pipe = piped(&["version"]);

    assert!(
        terminal.contains(concat!(" ", env!("CARGO_PKG_VERSION"), "\n")),
        "{terminal:?}"
    );
    assert!(terminal.contains("target"), "{terminal:?}");
    assert!(
        serde_json::from_str::<serde_json::Value>(&terminal).is_err(),
        "a person does not get JSON"
    );

    let json: serde_json::Value = serde_json::from_str(&pipe).unwrap();
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn a_terminal_gets_colour_unless_it_is_switched_off() {
    let escape = '\u{1b}';
    let colour = on_a_terminal(&["version"], &[("TERM", "xterm-256color")]);
    let no_color_variable = on_a_terminal(
        &["version"],
        &[("TERM", "xterm-256color"), ("NO_COLOR", "1")],
    );
    let no_color_flag = on_a_terminal(&["version", "--no-color"], &[("TERM", "xterm-256color")]);
    let dumb = on_a_terminal(&["version"], &[("TERM", "dumb")]);

    assert!(colour.contains(escape), "{colour:?}");
    for (what, output) in [
        ("NO_COLOR", no_color_variable),
        ("--no-color", no_color_flag),
        ("TERM=dumb", dumb),
    ] {
        assert!(!output.contains(escape), "{what}: {output:?}");
        assert!(output.starts_with("jev "), "{what}: {output:?}");
    }
}

#[cfg(feature = "internal-test-hooks")]
#[test]
fn an_evaluation_is_drawn_with_bars_on_a_terminal_and_is_the_envelope_on_a_pipe() {
    let response = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/response.json");
    let arguments = ["debug", "render", "--file", response];

    let unicode = on_a_terminal(
        &arguments,
        &[
            ("TERM", "xterm-256color"),
            ("LANG", "en_GB.UTF-8"),
            ("NO_COLOR", "1"),
        ],
    );
    let ascii = on_a_terminal(&arguments, &[("TERM", "xterm-256color"), ("NO_COLOR", "1")]);
    let pipe = piped(&arguments);

    assert!(
        unicode.contains("technical  0.85  █████████████████░░░"),
        "{unicode}"
    );
    assert!(unicode.contains(" · 312 input tokens · "), "{unicode}");
    assert!(
        ascii.is_ascii() && ascii.contains("technical  0.85  #################..."),
        "{ascii}"
    );
    let envelope: serde_json::Value = serde_json::from_str(&pipe).unwrap();
    assert_eq!(envelope["answers"]["department"]["choice"], "technical");
}
