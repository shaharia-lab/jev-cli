//! Unofficial Rust client for [TypeSafe AI](https://typesafe.ai)'s **Jev** model.
//!
//! Jev is a "System One" model: it never generates text. A request carries a `state` plus a map
//! of typed questions (`noul`, `choice`, `score`) and the response carries calibrated
//! probabilities over answers the caller defined.
//!
//! This crate is the library half of the [`jev` command-line tool]. It owns the typed request and
//! answer model, offline validation, the HTTP transport with retries, and typed errors. It
//! deliberately knows nothing about command-line parsing, terminals or configuration files, so it
//! can be used on its own as a Rust client.
//!
//! This project is not affiliated with, endorsed by, or sponsored by TypeSafe AI.
//!
//! [`jev` command-line tool]: https://github.com/shaharia-lab/jev-cli
#![forbid(unsafe_code)]

/// Version of this crate, as published.
///
/// The command-line tool reports it, and the HTTP transport will send it in the `User-Agent`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn version_is_a_semver_triple() {
        let core = VERSION.split(['-', '+']).next().unwrap();
        let parts: Vec<&str> = core.split('.').collect();

        assert_eq!(parts.len(), 3, "expected MAJOR.MINOR.PATCH, got {VERSION}");
        for part in parts {
            part.parse::<u64>()
                .unwrap_or_else(|_| panic!("non-numeric component in {VERSION}"));
        }
    }
}
