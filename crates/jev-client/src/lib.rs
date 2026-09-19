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
//! # The typed model
//!
//! [`Request`] holds a `state` and a map of [`Question`]s ([`Noul`], [`Choice`], [`Score`]);
//! [`Response`] holds one [`Answer`] per question plus [`Usage`]. `state`, `instructions` and
//! criteria values are free-form [`Content`]. [`ModelList`] is the body of `GET /v1/models`, and
//! [`pricing`] turns usage into an estimated cost.
//!
//! ```
//! use jev_client::{Noul, Request, Response, pricing};
//!
//! let request = Request::new("Help! My payouts have been failing for 3 days.", "jev-latest")
//!     .question("is_urgent", Noul::new("Does this convey urgency?"));
//! assert!(serde_json::to_string(&request).unwrap().contains(r#""type":"noul""#));
//!
//! let response: Response = serde_json::from_str(
//!     r#"{
//!         "model": "jev-1.13.0",
//!         "answers": { "is_urgent": { "type": "noul", "noul": 0.92 } },
//!         "usage": { "input_tokens": 312, "output_tokens": 48 }
//!     }"#,
//! )
//! .unwrap();
//!
//! let is_urgent = response.answers.get("is_urgent").and_then(|answer| answer.as_noul());
//! assert_eq!(is_urgent.map(|answer| answer.noul), Some(0.92));
//! assert!(pricing::estimate_cost_usd(&response.model, response.usage).is_some());
//! ```
//!
//! Three properties hold across these types:
//!
//! - **Requests are sent as written.** Key order is preserved, an explicit `null` stays distinct
//!   from an absent key, and fields this crate does not know are kept and passed through. They are
//!   kept rather than dropped because the API silently accepts an unknown field inside a question,
//!   so offline validation needs to see a misspelt `criteria` to report it.
//! - **Responses parse tolerantly.** Unknown fields are ignored, and an answer of an unknown type
//!   (or a known type whose shape has changed) is kept as raw JSON in [`Answer::Unknown`] instead
//!   of failing the whole response.
//! - **One definition, many uses.** Every type derives [`schemars::JsonSchema`], so the JSON
//!   Schemas handed to editors, AI agents and MCP clients cannot drift from what is parsed.
//!
//! The types describe shapes only. Limits such as 255 choice options or 2 to 10 score levels are
//! enforced by offline validation, not by parsing.
//!
//! This project is not affiliated with, endorsed by, or sponsored by TypeSafe AI.
//!
//! [`jev` command-line tool]: https://github.com/shaharia-lab/jev-cli
#![forbid(unsafe_code)]

mod answer;
mod content;
mod de;
mod models;
pub mod pricing;
mod question;
mod request;
mod response;

pub use answer::{Answer, ChoiceAnswer, NoulAnswer, ScoreAnswer};
pub use content::Content;
pub use models::{ModelCard, ModelList};
pub use question::{Choice, Noul, NoulCriteria, Question, Score};
pub use request::Request;
pub use response::{Response, Usage};

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
