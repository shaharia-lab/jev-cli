//! Diagnostics on stderr, chosen by `--verbose`, `--quiet` and `TYPESAFE_LOG_LEVEL`.

use std::io;

use tracing::level_filters::LevelFilter;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

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

/// Starts logging to stderr. Safe to call more than once: only the first call takes effect.
pub(crate) fn init(filter: Targets, color: bool) {
    let layer = tracing_subscriber::fmt::layer()
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
