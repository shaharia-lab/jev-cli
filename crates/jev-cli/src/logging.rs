//! Diagnostics on stderr, chosen by `--verbose`, `--quiet` and `TYPESAFE_LOG_LEVEL`.

use std::fmt;
use std::io;

use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::field::{MakeVisitor, RecordFields, VisitOutput};
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt::FormatFields;
use tracing_subscriber::fmt::format::{DefaultFields, Writer};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::output::printable;

/// The crates whose events `-v` shows. `-vv` and above show every crate's.
const OWN_TARGETS: [&str; 2] = ["jev_cli", "jev_client"];

/// What to log, given the flags and the environment variable the official SDKs share.
///
/// Bodies are not part of any level: `jev-client` only emits them when a caller opts in.
pub(crate) fn filter(verbose: u8, quiet: bool, sdk_level: Option<&str>) -> Targets {
    let own = |level: LevelFilter| {
        OWN_TARGETS.iter().fold(Targets::new(), |targets, target| {
            targets.with_target(*target, level)
        })
    };

    if quiet {
        return own(LevelFilter::ERROR);
    }
    match verbose {
        0 => own(sdk_level.and_then(parse_level).unwrap_or(LevelFilter::WARN)),
        1 => own(LevelFilter::DEBUG),
        2 => Targets::new().with_default(LevelFilter::DEBUG),
        _ => Targets::new().with_default(LevelFilter::TRACE),
    }
}

fn parse_level(level: &str) -> Option<LevelFilter> {
    match level.trim().to_ascii_lowercase().as_str() {
        "trace" => Some(LevelFilter::TRACE),
        "debug" => Some(LevelFilter::DEBUG),
        "info" => Some(LevelFilter::INFO),
        "warn" | "warning" => Some(LevelFilter::WARN),
        "error" => Some(LevelFilter::ERROR),
        "off" | "none" => Some(LevelFilter::OFF),
        _ => None,
    }
}

/// The usual `key=value` field format, with every recorded value passed through [`printable`].
///
/// A diagnostic line is the one place where text `jev` did not write itself reaches a terminal
/// `Display`-formatted — a response body under `--debug-bodies`, an error, the `method` of an MCP
/// request — and it is turned on precisely when something has already gone wrong. Neutralising
/// the values here holds the diagnostic stream to the same rule as the rest of human output, and
/// holds it for whatever a dependency logs too, which `-vv` turns on wholesale.
///
/// Values are neutralised, never dropped, so the line still shows what was there. Levels, targets
/// and `jev`'s own colours are unchanged, as are the machine output formats, which do not go
/// through a subscriber at all.
struct PrintableFields(DefaultFields);

impl<'writer> FormatFields<'writer> for PrintableFields {
    fn format_fields<R: RecordFields>(&self, writer: Writer<'writer>, fields: R) -> fmt::Result {
        let mut visitor = self.0.make_visitor(writer);
        fields.record(&mut Neutralise(&mut visitor));
        visitor.finish()
    }
}

/// A visitor that hands each value to `V` with its control characters written out.
///
/// Only `record_str` and `record_debug` are implemented: every other `record_*` method, including
/// `record_error`, defaults to `record_debug`, so none of them can get around the escaping. The
/// string `record_debug` renders costs nothing when logging is off, which is the default.
struct Neutralise<'visitor, V>(&'visitor mut V);

impl<V: Visit> Visit for Neutralise<'_, V> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.record_str(field, &printable(value));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        // `Debug` for `Arguments` writes the text as it stands, so the inner visitor formats
        // exactly what it would have, minus the control characters.
        let shown = format!("{value:?}");
        self.0
            .record_debug(field, &format_args!("{}", printable(&shown)));
    }
}

/// Starts logging to stderr. Safe to call more than once: only the first call takes effect.
pub(crate) fn init(filter: Targets, color: bool) {
    let layer = tracing_subscriber::fmt::layer()
        .fmt_fields(PrintableFields(DefaultFields::new()))
        .with_writer(io::stderr)
        .with_ansi(color)
        .with_target(true)
        .without_time();
    let _ = tracing_subscriber::registry()
        .with(layer)
        .with(filter)
        .try_init();
}

#[cfg(test)]
mod tests {
    use tracing::Level;

    use super::filter;

    #[test]
    fn verbosity_widens_from_warnings_to_everything() {
        let quiet = filter(0, true, None);
        let default = filter(0, false, None);
        let verbose = filter(1, false, None);
        let very_verbose = filter(2, false, None);

        assert!(!quiet.would_enable("jev_client", &Level::WARN));
        assert!(quiet.would_enable("jev_client", &Level::ERROR));
        assert!(default.would_enable("jev_client", &Level::WARN));
        assert!(!default.would_enable("jev_client", &Level::DEBUG));
        assert!(verbose.would_enable("jev_client::http", &Level::DEBUG));
        assert!(
            !verbose.would_enable("hyper", &Level::DEBUG),
            "-v is about jev, not its HTTP stack"
        );
        assert!(very_verbose.would_enable("hyper", &Level::DEBUG));
        assert!(filter(3, false, None).would_enable("hyper", &Level::TRACE));
    }

    #[test]
    fn the_sdk_log_level_variable_is_honoured_until_a_flag_says_otherwise() {
        assert!(filter(0, false, Some("debug")).would_enable("jev_client", &Level::DEBUG));
        assert!(!filter(0, false, Some("error")).would_enable("jev_client", &Level::WARN));
        assert!(filter(0, false, Some("nonsense")).would_enable("jev_client", &Level::WARN));
        assert!(
            !filter(0, true, Some("debug")).would_enable("jev_client", &Level::DEBUG),
            "--quiet wins"
        );
    }
}
