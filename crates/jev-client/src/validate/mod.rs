//! Offline validation of a request, before any money is spent on it.
//!
//! The API does not enforce its own limits reliably: a one-level score returns 200 with a
//! meaningless answer, an eleven-level score fails with a server error, and an unknown field inside
//! a question is silently ignored. Checking a request here is therefore a matter of correctness,
//! not convenience. Nothing in this module performs I/O.
//!
//! A request is checked as a [`Document`], the raw JSON it was written as, so that every problem
//! can be reported at once, each as a [`Finding`] that names the question, points at the offending
//! part, says which [`Rule`] was broken and suggests a fix. A typed [`Request`] can be checked too.
//!
//! ```
//! use jev_client::validate::{self, Document, Options, Rule};
//!
//! let document = Document::from_json_str(
//!     r#"{
//!         "state": "Help! My payouts have been failing for 3 days.",
//!         "model": "jev-latest",
//!         "questions": {
//!             "frustration": { "type": "score", "instructions": "How frustrated?", "criteria": ["Angry"] }
//!         }
//!     }"#,
//! )
//! .unwrap();
//!
//! let report = validate::check_document(&document, &Options::default());
//!
//! assert!(!report.is_valid());
//! let finding = report.errors().next().unwrap();
//! assert_eq!(finding.rule, Rule::ScoreTooFewLevels);
//! assert_eq!(finding.question.as_deref(), Some("frustration"));
//! assert_eq!(finding.path, "/questions/frustration/criteria");
//! ```

mod document;
mod finding;
mod lints;
mod rules;
mod size;

pub use document::{Document, DuplicateKey};
pub use finding::{Finding, Report, Rule, Severity};
pub use size::{
    OVERHEAD_TOKENS, QUESTION_BUDGET_TOKENS, SizeEstimate, TOTAL_BUDGET_TOKENS,
    estimate_text_tokens, estimate_value_tokens,
};

use crate::request::Request;

/// The most options a choice may have.
pub const MAX_CHOICE_OPTIONS: usize = 255;

/// The fewest levels a score may have.
pub const MIN_SCORE_LEVELS: usize = 2;

/// The most levels a score may have.
pub const MAX_SCORE_LEVELS: usize = 10;

/// How a request is validated.
///
/// ```
/// use jev_client::validate::Options;
///
/// let options = Options::default().strict(true).skip_size_check(true);
/// assert!(options.is_strict());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Options {
    strict: bool,
    skip_size_check: bool,
    model_optional: bool,
}

impl Options {
    /// In strict mode every warning becomes an error, so a request with any finding is invalid.
    #[must_use]
    pub const fn strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    /// Skips the size estimate, for a caller who knows better than a heuristic.
    #[must_use]
    pub const fn skip_size_check(mut self, skip: bool) -> Self {
        self.skip_size_check = skip;
        self
    }

    /// Accepts a request without a `model`, for a request file whose model is decided elsewhere.
    /// A `model` that is present must still be a non-empty string.
    #[must_use]
    pub const fn model_optional(mut self, optional: bool) -> Self {
        self.model_optional = optional;
        self
    }

    /// Whether warnings are promoted to errors.
    #[must_use]
    pub const fn is_strict(&self) -> bool {
        self.strict
    }
}

/// Validates a request as it was written.
#[must_use]
pub fn check_document(document: &Document, options: &Options) -> Report {
    let mut findings = Vec::new();
    let mut size = None;

    if let Some(request) = rules::check_shape(document, options, &mut findings) {
        lints::check(&request, &mut findings);
        if !options.skip_size_check {
            let estimate = SizeEstimate::of(
                request.state,
                request.questions.iter().map(|q| (q.id, q.value)),
            );
            rules::check_size(&estimate, &mut findings);
            size = Some(estimate);
        }
    }

    if options.strict {
        for finding in &mut findings {
            finding.severity = Severity::Error;
        }
    }
    Report { findings, size }
}

/// Validates a typed request. Duplicate keys cannot occur in one, so those rules never fire.
#[must_use]
pub fn check(request: &Request, options: &Options) -> Report {
    check_document(&Document::from(request), options)
}
