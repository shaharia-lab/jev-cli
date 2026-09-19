//! An offline estimate of how many input tokens a request will use.

use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

/// Tokens the API adds to every request, whatever its content. Measured at 258 to 273.
pub const OVERHEAD_TOKENS: u64 = 270;

/// The budget for the state plus every question.
pub const TOTAL_BUDGET_TOKENS: u64 = 64_000;

/// The budget for the state plus the single longest question.
pub const QUESTION_BUDGET_TOKENS: u64 = 32_000;

/// A run of ASCII letters no longer than this is counted as a single token.
const SHORT_WORD_LETTERS: u64 = 8;

/// Letters per token in a longer run of ASCII letters.
const LONG_WORD_LETTERS_PER_TOKEN: u64 = 4;

/// Characters per token in a run of non-ASCII alphabetic characters.
const NON_ASCII_LETTERS_PER_TOKEN: u64 = 3;

/// From this code point upwards (CJK radicals, kana, ideographs, Hangul), one character is one token.
const FIRST_TOKEN_PER_CHARACTER: char = '\u{2E80}';

/// The estimated size of a request, in input tokens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[non_exhaustive]
pub struct SizeEstimate {
    /// Tokens in `state`.
    pub state_tokens: u64,
    /// Tokens in all the questions together.
    pub questions_tokens: u64,
    /// The fixed per-request overhead included in both totals below.
    pub overhead_tokens: u64,
    /// Estimated input tokens for the whole request, to compare with `total_budget_tokens`.
    pub total_tokens: u64,
    /// The budget for the whole request.
    pub total_budget_tokens: u64,
    /// Estimated tokens for the state plus the longest question, to compare with
    /// `question_budget_tokens`.
    pub largest_question_tokens: u64,
    /// The id of the longest question.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub largest_question: Option<String>,
    /// The budget for the state plus the longest question.
    pub question_budget_tokens: u64,
}

impl SizeEstimate {
    /// Estimates a request from its `state` and its questions, each given with its id.
    pub(crate) fn of<'a>(
        state: &Value,
        questions: impl Iterator<Item = (&'a str, &'a Value)>,
    ) -> Self {
        let state_tokens = estimate_value_tokens(state);
        let mut questions_tokens = 0_u64;
        let mut largest: Option<(&str, u64)> = None;
        for (id, question) in questions {
            let tokens = estimate_value_tokens(question);
            questions_tokens = questions_tokens.saturating_add(tokens);
            if largest.is_none_or(|(_, most)| tokens > most) {
                largest = Some((id, tokens));
            }
        }

        let base = OVERHEAD_TOKENS.saturating_add(state_tokens);
        Self {
            state_tokens,
            questions_tokens,
            overhead_tokens: OVERHEAD_TOKENS,
            total_tokens: base.saturating_add(questions_tokens),
            total_budget_tokens: TOTAL_BUDGET_TOKENS,
            largest_question_tokens: base.saturating_add(largest.map_or(0, |(_, tokens)| tokens)),
            largest_question: largest.map(|(id, _)| id.to_owned()),
            question_budget_tokens: QUESTION_BUDGET_TOKENS,
        }
    }
}

/// Estimates the input tokens of a JSON value: a string as text, anything else from its compact
/// JSON form. See [`estimate_text_tokens`] for how, and how well.
#[must_use]
pub fn estimate_value_tokens(value: &Value) -> u64 {
    match value {
        // A bare string is sent as text, not as a quoted JSON string.
        Value::String(text) => estimate_text_tokens(text),
        other => serde_json::to_string(other).map_or(0, |json| estimate_text_tokens(&json)),
    }
}

/// Estimates the input tokens of a piece of text.
///
/// The API has two budgets for `jev-1.13.0`: 64k tokens for the state plus every question, and 32k
/// for the state plus the single longest question. There is no tokeniser to call offline, so the
/// size is estimated from the text itself.
///
/// A flat characters-per-token ratio does not work: measured against the live API, English prose
/// runs at about 6.5 characters per token and an array of numbers at 1.0. What does hold, across
/// every kind of content tried, is this:
///
/// - a run of ASCII letters of up to 8 characters is one token, and a longer run (a rare word, an
///   identifier) about one token per 4 letters;
/// - a run of other alphabetic characters is about one token per 3 (accented Latin, Cyrillic,
///   Greek), except CJK and similar scripts, where every character is a token;
/// - every digit and every punctuation character is a token of its own;
/// - a single space is free, but every line break is a token, and so is a run of two or more
///   spaces or tabs (indentation).
///
/// Structured content is measured over its compact JSON form. Against real token counts for a
/// dozen kinds of content (prose, records, numbers, chat logs, identifiers, hashes, Japanese,
/// French, indented text) this lands within 15% every time, and within 5% overall. It remains an
/// estimate, and every finding that uses it says so.
///
/// ```
/// use jev_client::validate::estimate_text_tokens;
///
/// assert_eq!(estimate_text_tokens("payouts have been failing"), 4);
/// assert_eq!(estimate_text_tokens("2026-09-19"), 10);
/// ```
#[must_use]
pub fn estimate_text_tokens(text: &str) -> u64 {
    let mut tokens = 0_u64;
    let mut word = Run::default();
    let mut gap = Gap::default();
    for character in text.chars() {
        if character.is_whitespace() {
            tokens = tokens.saturating_add(word.take_tokens());
            gap.push(character);
            continue;
        }
        tokens = tokens.saturating_add(gap.take_tokens());
        if character.is_alphabetic() {
            word.push(character);
        } else {
            tokens = tokens.saturating_add(word.take_tokens()).saturating_add(1);
        }
    }
    tokens
        .saturating_add(word.take_tokens())
        .saturating_add(gap.take_tokens())
}

/// A run of consecutive whitespace.
#[derive(Default)]
struct Gap {
    line_breaks: u64,
    blanks: u64,
}

impl Gap {
    fn push(&mut self, character: char) {
        if character == '\n' {
            self.line_breaks += 1;
        } else if character != '\r' {
            self.blanks += 1;
        }
    }

    /// The tokens in the gap, which is then reset: one per line break, and one for indentation.
    fn take_tokens(&mut self) -> u64 {
        let gap = std::mem::take(self);
        gap.line_breaks + u64::from(gap.blanks >= 2)
    }
}

/// A run of consecutive alphabetic characters.
#[derive(Default)]
struct Run {
    ascii_letters: u64,
    other_letters: u64,
    token_per_character: u64,
}

impl Run {
    fn push(&mut self, character: char) {
        if character.is_ascii() {
            self.ascii_letters += 1;
        } else if character >= FIRST_TOKEN_PER_CHARACTER {
            self.token_per_character += 1;
        } else {
            self.other_letters += 1;
        }
    }

    /// The tokens in the run, which is then reset.
    fn take_tokens(&mut self) -> u64 {
        let run = std::mem::take(self);
        let letters = run.ascii_letters + run.other_letters;
        let words = if letters == 0 {
            0
        } else if run.other_letters > 0 {
            letters.div_ceil(NON_ASCII_LETTERS_PER_TOKEN)
        } else if letters <= SHORT_WORD_LETTERS {
            1
        } else {
            letters.div_ceil(LONG_WORD_LETTERS_PER_TOKEN)
        };
        words + run.token_per_character
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{OVERHEAD_TOKENS, SizeEstimate, estimate_text_tokens, estimate_value_tokens};

    #[test]
    fn counts_words_digits_and_punctuation() {
        assert_eq!(estimate_text_tokens(""), 0);
        assert_eq!(estimate_text_tokens("a b"), 2, "a single space is free");
        assert_eq!(estimate_text_tokens("a\nb"), 3, "a line break is a token");
        assert_eq!(
            estimate_text_tokens("a\r\n    b"),
            4,
            "and so is the indentation after it"
        );
        assert_eq!(
            estimate_text_tokens("a  b"),
            3,
            "a run of spaces counts once"
        );
        assert_eq!(estimate_text_tokens("payouts have been failing"), 4);
        assert_eq!(
            estimate_text_tokens("3 days!"),
            3,
            "a digit, a word, a punctuation mark"
        );
        assert_eq!(
            estimate_text_tokens("2026-09-19"),
            10,
            "every digit and dash counts"
        );
        assert_eq!(estimate_text_tokens(r#"{"id":42}"#), 8);
    }

    #[test]
    fn long_words_and_identifiers_cost_more_than_short_ones() {
        assert_eq!(estimate_text_tokens("customer"), 1, "8 letters");
        assert_eq!(
            estimate_text_tokens("customers"),
            3,
            "9 letters, a token per 4"
        );
        assert_eq!(estimate_text_tokens("electroencephalography"), 6);
        assert_eq!(estimate_text_tokens("getUserAccountById"), 5);
        assert_eq!(
            estimate_text_tokens("retry_after_ms"),
            5,
            "three words and two underscores"
        );
    }

    #[test]
    fn other_scripts_are_denser_than_english() {
        assert_eq!(
            estimate_text_tokens("支払いが失敗"),
            6,
            "one token per character"
        );
        assert_eq!(
            estimate_text_tokens("échouent"),
            3,
            "accented Latin: a token per 3 letters"
        );
        assert_eq!(estimate_text_tokens("платеж"), 2);
        assert_eq!(
            estimate_text_tokens("paid支払い"),
            4,
            "a mixed run counts both parts"
        );
    }

    #[test]
    fn a_bare_string_is_counted_as_text_and_structure_as_compact_json() {
        assert_eq!(
            estimate_value_tokens(&json!("two words")),
            2,
            "no quotes are counted"
        );
        assert_eq!(estimate_value_tokens(&json!(["a", "b"])), 9, r#"["a","b"]"#);
        assert_eq!(estimate_value_tokens(&json!(null)), 1);
    }

    #[test]
    fn a_request_estimate_adds_the_overhead_and_finds_the_longest_question() {
        let state = json!("payouts have been failing");
        let short = json!({ "type": "noul", "instructions": "Urgent?" });
        let long =
            json!({ "type": "noul", "instructions": "Does the customer sound urgent today?" });
        let short_tokens = estimate_value_tokens(&short);
        let long_tokens = estimate_value_tokens(&long);

        let estimate = SizeEstimate::of(&state, [("short", &short), ("long", &long)].into_iter());

        assert_eq!(estimate.state_tokens, 4);
        assert_eq!(estimate.questions_tokens, short_tokens + long_tokens);
        assert_eq!(
            estimate.total_tokens,
            OVERHEAD_TOKENS + 4 + short_tokens + long_tokens
        );
        assert_eq!(estimate.largest_question.as_deref(), Some("long"));
        assert_eq!(
            estimate.largest_question_tokens,
            OVERHEAD_TOKENS + 4 + long_tokens
        );
    }

    #[test]
    fn a_request_without_questions_still_has_an_estimate() {
        let estimate = SizeEstimate::of(&json!("hi"), std::iter::empty());

        assert_eq!(estimate.largest_question, None);
        assert_eq!(estimate.total_tokens, estimate.largest_question_tokens);
    }
}
