//! The `jev` command-line tool. This library exists for the `jev` binary and for tooling that
//! needs its command tree (documentation, completions); it is not a stable API.
//!
//! The library is [`jev_client`]: that is the crate to build on.
#![forbid(unsafe_code)]

mod batch;
mod cli;
mod client;
mod commands;
mod config;
mod credentials;
mod duration;
mod env;
mod error;
mod evaluate;
mod exit;
#[cfg(any(test, fuzzing))]
#[doc(hidden)]
pub mod fuzz;
mod gate;
mod help;
mod input;
mod interaction;
mod interrupt;
mod logging;
mod man;
mod notice;
mod output;
mod schemas;
mod suggest;
mod update;

use std::io::{self, IsTerminal, Write};
use std::panic::{self, AssertUnwindSafe};
use std::process::ExitCode;

use clap::FromArgMatches;
use clap::error::ErrorKind as ClapErrorKind;

use crate::cli::Cli;
use crate::client::Connection;
use crate::commands::{Configuration, Context};
use crate::config::{ConfigStore, Flags, Settings};
use crate::env::Env;
use crate::error::CliError;
use crate::exit::Exit;
use crate::interaction::Interaction;
use crate::notice::Notifier;
use crate::output::{Format, Output, Ui};

pub use crate::commands::completion::Shell;

/// The command tree with its full help, for tooling that generates documentation or completions
/// from it.
#[must_use]
pub fn command() -> clap::Command {
    cli::command()
}

/// The completion script `jev completion <shell>` prints, for packaging it with a release.
#[must_use]
pub fn completion_script(shell: Shell) -> Vec<u8> {
    commands::completion::script(shell)
}

/// Writes the man pages, `jev.1` and one per command such as `jev-auth-login.1`, into
/// `directory`, for packaging them with a release.
///
/// # Errors
///
/// When a page cannot be written.
pub fn write_man_pages(directory: &std::path::Path) -> std::io::Result<()> {
    man::write_all(directory)
}

/// Runs `jev` with the process's arguments, environment and standard streams.
///
/// A panic anywhere inside is caught and reported as an internal error with exit code 1, never as
/// Rust's own exit code 101: the exit codes are a contract.
#[must_use]
pub fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args_os()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    let terminal = Terminal::detect();
    // Known before anything is parsed, so that even a parse error or a crash is reported in the
    // format the caller asked for.
    let early_format = Format::resolve(
        Format::scan(&arguments).or_else(|| {
            env("JEV_OUTPUT")
                .and_then(|value| <Format as clap::ValueEnum>::from_str(&value, true).ok())
        }),
        terminal.stdout,
    );
    let no_color = arguments.iter().any(|argument| argument == "--no-color");
    let ascii = arguments.iter().any(|argument| argument == "--ascii");
    let stderr_ui = terminal.ui(terminal.stderr, no_color, ascii);

    panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map_or_else(|| "unknown location".to_owned(), ToString::to_string);
        let error = CliError::internal(format!("jev crashed at {location}"));
        report(&error, early_format, stderr_ui);
    }));

    let outcome = panic::catch_unwind(AssertUnwindSafe(|| run(&arguments, terminal, early_format)));
    match outcome {
        // Not only success: a gate that is false is exit 10, and that is not an error.
        Ok(Ok(exit)) => exit.into(),
        Ok(Err(failure)) => {
            if let Some(error) = failure.error {
                report(&error, failure.format, stderr_ui);
            }
            failure.exit.into()
        }
        Err(_) => Exit::Internal.into(),
    }
}

/// How a run ended when it did not succeed.
struct Failure {
    exit: Exit,
    /// `None` when there is nothing left to print (clap has already printed help, say).
    /// Boxed so that the error path does not widen every `Result` on the way.
    error: Option<Box<CliError>>,
    format: Format,
}

fn run(arguments: &[String], terminal: Terminal, early_format: Format) -> Result<Exit, Failure> {
    // Parsed with the help attached, so that `--help` shows it.
    let cli = cli::command()
        .try_get_matches_from(arguments)
        .and_then(|matches| {
            Cli::from_arg_matches(&matches).map_err(|error| error.format(&mut cli::command()))
        })
        .map_err(|error| parse_failure(&error, early_format))?;
    let global = &cli.global;
    let env = Env::from_process();

    let flags = Flags {
        profile: global.profile.clone(),
        base_url: global.base_url.clone(),
        model: global.model.clone(),
        output: global.output.or_else(|| Format::scan(arguments)),
        timeout: global.timeout,
        max_retries: global.max_retries,
        concurrency: cli.command.concurrency(),
    };
    let store = config::config_dir(&env).map(ConfigStore::new);
    let loaded = store.clone().and_then(|store| {
        let file = store.load()?;
        let settings =
            Settings::resolve(&flags, &env, file.active_profile.as_deref(), &file.profiles)?;
        Ok((file, settings))
    });

    // The profile may set the format. When the configuration is broken, what was worked out
    // before parsing still stands, so the error about it is printed the way the caller asked.
    let format = match &loaded {
        Ok((_, settings)) => Format::resolve(settings.output(), terminal.stdout),
        Err(_) => early_format,
    };
    let fail = |error: CliError| Failure {
        exit: error.exit,
        error: Some(Box::new(error)),
        format,
    };

    let stderr_ui = terminal.ui(terminal.stderr, global.no_color, global.ascii);
    logging::init(
        logging::filter(global.verbose, global.quiet, env.get("TYPESAFE_LOG_LEVEL")),
        stderr_ui.has_color(),
    );
    let notifier = Notifier {
        format,
        ui: stderr_ui,
        quiet: global.quiet,
        // A dumb terminal cannot erase a line, so nothing is redrawn on it.
        terminal: terminal.stderr && !terminal.dumb,
    };
    if let Ok((file, _)) = &loaded {
        for warning in &file.warnings {
            notifier.emit(warning);
        }
    }

    let stdout = io::stdout();
    // Not locked for the whole run: `jev batch run` reads piped rows on a thread of its own.
    let mut stdin = io::stdin();
    let mut context = Context {
        output: Output {
            format,
            field: global.field.clone(),
            ui: terminal.ui(terminal.stdout, global.no_color, global.ascii),
        },
        interaction: Interaction::new(terminal.stdin, global.no_input, env.get("CI")),
        connection: Connection {
            insecure_allow_http: global.insecure_allow_http,
            debug_bodies: global.debug_bodies,
        },
        notifier,
        configuration: Configuration {
            store,
            loaded,
            flags,
        },
        env,
        stdout: &mut stdout.lock(),
        stdin: &mut stdin,
    };
    commands::run(&cli.command, &mut context).map_err(fail)
}

/// Turns a `clap` error into help on stdout, or a usage error on stderr.
fn parse_failure(error: &clap::Error, format: Format) -> Failure {
    if matches!(
        error.kind(),
        ClapErrorKind::DisplayHelp | ClapErrorKind::DisplayVersion
    ) {
        // Asked-for help is data: it goes to stdout and the run succeeded.
        let _ = error.print();
        return Failure {
            exit: Exit::Success,
            error: None,
            format,
        };
    }
    if error.kind() == ClapErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        && format.is_machine_readable()
    {
        // For a person `clap` prints the help, below. For a program the help text is noise.
        let usage = CliError::usage("no command was given").hint(
            "run `jev --help` to see the commands, or `jev <command> --help` for one of them",
        );
        return Failure {
            exit: Exit::Usage,
            error: Some(Box::new(usage)),
            format,
        };
    }
    if !format.is_machine_readable() {
        // `clap` renders usage, the nearest valid command or flag, and a pointer to --help.
        let _ = error.print();
        return Failure {
            exit: Exit::Usage,
            error: None,
            format,
        };
    }

    let rendered = error.render().to_string();
    let mut lines = rendered
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let message = lines
        .next()
        .unwrap_or("invalid command line")
        .trim_start_matches("error: ")
        .to_owned();
    let hint: Vec<&str> = lines
        .filter(|line| line.starts_with("tip:") || line.starts_with("Usage:"))
        .collect();
    let mut usage = CliError::usage(message);
    usage = if hint.is_empty() {
        usage.hint("run `jev --help` to see the commands and flags")
    } else {
        usage.hint(hint.join("; "))
    };
    Failure {
        exit: Exit::Usage,
        error: Some(Box::new(usage)),
        format,
    }
}

/// Prints an error on stderr: a JSON object for a program, text for a person.
fn report(error: &CliError, format: Format, ui: Ui) {
    let text = if format.is_machine_readable() {
        error.to_json().to_string()
    } else {
        error.to_human(ui)
    };
    let _ = writeln!(io::stderr().lock(), "{text}");
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// Which standard streams are terminals, and what the environment says about them.
// Six independent facts about the environment, each of them a yes or a no.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy)]
struct Terminal {
    stdin: bool,
    stdout: bool,
    stderr: bool,
    no_color: bool,
    dumb: bool,
    utf8: bool,
}

impl Terminal {
    fn detect() -> Self {
        Self {
            stdin: io::stdin().is_terminal(),
            stdout: io::stdout().is_terminal(),
            stderr: io::stderr().is_terminal(),
            // https://no-color.org: present and not empty, whatever the value.
            no_color: env("NO_COLOR").is_some(),
            dumb: env("TERM").is_some_and(|term| term == "dumb"),
            utf8: locale_is_utf8(
                env("LC_ALL")
                    .or_else(|| env("LC_CTYPE"))
                    .or_else(|| env("LANG"))
                    .as_deref(),
            ),
        }
    }

    /// The look of human output on a stream that is, or is not, a terminal.
    fn ui(self, stream_is_terminal: bool, no_color_flag: bool, ascii_flag: bool) -> Ui {
        let color = stream_is_terminal && !self.no_color && !no_color_flag && !self.dumb;
        Ui::new(color, self.utf8 && !ascii_flag)
    }
}

/// Whether the locale can show Unicode. Windows terminals can; elsewhere the locale says so, and
/// an unset locale means the `C` locale, which cannot.
fn locale_is_utf8(locale: Option<&str>) -> bool {
    if cfg!(windows) {
        return true;
    }
    locale.is_some_and(|locale| {
        let locale = locale.to_ascii_lowercase();
        locale.contains("utf-8") || locale.contains("utf8")
    })
}

#[cfg(test)]
mod tests {
    use super::{Terminal, locale_is_utf8};

    fn terminal(no_color: bool, dumb: bool, utf8: bool) -> Terminal {
        Terminal {
            stdin: true,
            stdout: true,
            stderr: true,
            no_color,
            dumb,
            utf8,
        }
    }

    #[test]
    fn colour_needs_a_terminal_and_nothing_asking_for_it_to_be_off() {
        assert!(
            terminal(false, false, true)
                .ui(true, false, false)
                .has_color()
        );
        assert!(
            !terminal(false, false, true)
                .ui(false, false, false)
                .has_color(),
            "not a terminal"
        );
        assert!(
            !terminal(true, false, true)
                .ui(true, false, false)
                .has_color(),
            "NO_COLOR"
        );
        assert!(
            !terminal(false, true, true)
                .ui(true, false, false)
                .has_color(),
            "TERM=dumb"
        );
        assert!(
            !terminal(false, false, true)
                .ui(true, true, false)
                .has_color(),
            "--no-color"
        );
    }

    #[test]
    fn unicode_needs_a_utf8_locale_and_no_ascii_flag() {
        assert!(
            terminal(false, false, true)
                .ui(true, false, false)
                .has_unicode()
        );
        assert!(
            !terminal(false, false, false)
                .ui(true, false, false)
                .has_unicode()
        );
        assert!(
            !terminal(false, false, true)
                .ui(true, false, true)
                .has_unicode()
        );
    }

    #[test]
    fn the_locale_decides_whether_unicode_is_safe() {
        assert!(locale_is_utf8(Some("en_GB.UTF-8")));
        assert!(locale_is_utf8(Some("C.utf8")));

        // Windows terminals show Unicode whatever these variables say; elsewhere they decide.
        let without_a_utf8_locale = cfg!(windows);
        assert_eq!(locale_is_utf8(Some("C")), without_a_utf8_locale);
        assert_eq!(
            locale_is_utf8(Some("en_US.ISO-8859-1")),
            without_a_utf8_locale
        );
        assert_eq!(locale_is_utf8(None), without_a_utf8_locale);
    }
}
