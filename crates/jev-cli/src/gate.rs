//! Gates: turning an answer into an exit code, so a shell `if` can branch on it.
//!
//! A gate never turns a failure into "false". Exit 10 means exactly one thing: the evaluation
//! succeeded and the condition does not hold. Anything else that goes wrong, including an answer
//! that cannot be compared, is an error with its own exit code.

use std::fmt;

use indexmap::IndexMap;
use jev_client::{Question, Request};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

use crate::error::CliError;
use crate::exit::Exit;

/// A comparison operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    Ge,
    Le,
    Gt,
    Lt,
    Eq,
    Ne,
}

impl Op {
    /// Two-character operators first, so that `>=` is not read as `>`.
    const ALL: [(&'static str, Self); 6] = [
        (">=", Self::Ge),
        ("<=", Self::Le),
        ("==", Self::Eq),
        ("!=", Self::Ne),
        (">", Self::Gt),
        ("<", Self::Lt),
    ];

    const fn symbol(self) -> &'static str {
        match self {
            Self::Ge => ">=",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Lt => "<",
            Self::Eq => "==",
            Self::Ne => "!=",
        }
    }

    fn holds(self, actual: f64, expected: f64) -> bool {
        match self {
            Self::Ge => actual >= expected,
            Self::Le => actual <= expected,
            Self::Gt => actual > expected,
            Self::Lt => actual < expected,
            // Probabilities arrive rounded to two decimals; compare with a tolerance below that.
            Self::Eq => (actual - expected).abs() < 1e-9,
            Self::Ne => (actual - expected).abs() >= 1e-9,
        }
    }
}

/// What a condition expects.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Expected {
    /// A number compared with a noul, a score, or a confidence.
    Number(Op, f64),
    /// The chosen option is, or is not, this one.
    Option(Op, String),
    /// The chosen option is any of these.
    AnyOf(Vec<String>),
}

/// One condition on one answer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Condition {
    /// The question id.
    pub(crate) id: String,
    /// Whether the condition is on the answer's `confidence` rather than its value.
    pub(crate) on_confidence: bool,
    pub(crate) expected: Expected,
}

impl fmt::Display for Condition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let subject = if self.on_confidence {
            format!("{}.confidence", self.id)
        } else {
            self.id.clone()
        };
        match &self.expected {
            Expected::Number(op, number) => write!(formatter, "{subject} {} {number}", op.symbol()),
            Expected::Option(op, option) => write!(formatter, "{subject} {} {option}", op.symbol()),
            Expected::AnyOf(options) => write!(formatter, "{subject} in [{}]", options.join(", ")),
        }
    }
}

impl Condition {
    pub(crate) fn number(id: &str, on_confidence: bool, op: Op, number: f64) -> Self {
        Self {
            id: id.to_owned(),
            on_confidence,
            expected: Expected::Number(op, number),
        }
    }

    pub(crate) fn any_of(id: &str, options: Vec<String>) -> Self {
        Self {
            id: id.to_owned(),
            on_confidence: false,
            expected: Expected::AnyOf(options),
        }
    }

    /// Parses `--assert '<id> <op> <value>'`, such as `is_urgent >= 0.7`, `team == billing` or
    /// `team.confidence >= 0.8`. The value is a number, or an option name for `==` and `!=`.
    ///
    /// # Errors
    ///
    /// A usage error that shows the expected form.
    pub(crate) fn parse(text: &str) -> Result<Self, CliError> {
        let invalid = |problem: &str| {
            CliError::usage(format!("--assert `{text}` is not a condition: {problem}"))
                .hint("write `<question-id> <op> <value>`, e.g. `is_urgent >= 0.7`, `team == billing` or `team.confidence >= 0.8`; operators: >= <= > < == !=")
        };
        let (position, symbol, op) = Op::ALL
            .iter()
            .filter_map(|(symbol, op)| text.find(symbol).map(|position| (position, *symbol, *op)))
            // The leftmost operator, and at the same place the longer one.
            .min_by_key(|(position, symbol, _)| (*position, std::cmp::Reverse(symbol.len())))
            .ok_or_else(|| invalid("there is no operator"))?;
        let (subject, value) = (
            text.get(..position).unwrap_or_default().trim(),
            text.get(position + symbol.len()..)
                .unwrap_or_default()
                .trim(),
        );
        if subject.is_empty() || value.is_empty() {
            return Err(invalid("a question id and a value are both needed"));
        }

        let (id, on_confidence) = subject
            .strip_suffix(".confidence")
            .map_or((subject, false), |id| (id, true));
        let expected = match value.parse::<f64>() {
            Ok(number) if number.is_finite() => Expected::Number(op, number),
            _ if matches!(op, Op::Eq | Op::Ne) && !on_confidence => {
                Expected::Option(op, value.to_owned())
            }
            _ => {
                return Err(invalid(
                    "only == and != can compare with an option name; the others need a number",
                ));
            }
        };
        Ok(Self {
            id: id.to_owned(),
            on_confidence,
            expected,
        })
    }

    /// Checks the condition against the request, before anything is sent: the question must
    /// exist, be of a type the condition makes sense for, and an expected option must be one of
    /// its options.
    ///
    /// # Errors
    ///
    /// A usage error naming what does not fit.
    pub(crate) fn check_request(&self, request: &Request) -> Result<(), CliError> {
        let mismatch = |problem: String| {
            CliError::usage(format!(
                "the condition `{self}` cannot be checked: {problem}"
            ))
        };
        let Some(question) = request.questions.get(&self.id) else {
            let ids: Vec<&str> = request.questions.keys().map(String::as_str).collect();
            return Err(mismatch(format!("there is no question `{}`", self.id))
                .hint(format!("the questions are: {}", ids.join(", "))));
        };
        match (question, &self.expected, self.on_confidence) {
            (Question::Noul(_), _, true) => Err(mismatch(format!(
                "`{}` is a noul, and a noul has no confidence",
                self.id
            ))
            .hint("compare the probability itself: a value near 0.5 means the model cannot tell")),
            (Question::Choice(_), Expected::Number(..), false) => Err(mismatch(format!(
                "`{}` is a choice, whose answer is an option, not a number",
                self.id
            ))
            .hint(format!(
                "compare with an option (`{} == <option>`), or use `{}.confidence`",
                self.id, self.id
            ))),
            (
                Question::Noul(_) | Question::Score(_),
                Expected::Option(..) | Expected::AnyOf(_),
                _,
            ) => Err(mismatch(format!(
                "`{}` is answered with a number, not an option",
                self.id
            ))
            .hint("compare it with a number")),
            (Question::Choice(choice), Expected::Option(_, option), _)
                if !choice.criteria.contains_key(option) =>
            {
                Err(unknown_option(
                    &mismatch(String::new()),
                    option,
                    choice.criteria.keys(),
                ))
            }
            (Question::Choice(choice), Expected::AnyOf(options), _) => match options
                .iter()
                .find(|option| !choice.criteria.contains_key(*option))
            {
                Some(option) => Err(unknown_option(
                    &mismatch(String::new()),
                    option,
                    choice.criteria.keys(),
                )),
                None => Ok(()),
            },
            _ => Ok(()),
        }
    }

    /// Decides the condition against the answer the API gave.
    fn decide(&self, answer: Option<&Value>) -> Result<Decision, CliError> {
        let unreadable = |why: &str| {
            CliError::internal(format!("the condition `{self}` could not be decided: {why}"))
                .hint("the API answered in a way this version of jev does not understand; run `jev update`")
        };
        let answer =
            answer.ok_or_else(|| unreadable("the API returned no answer for that question"))?;
        let field = if self.on_confidence {
            "confidence"
        } else {
            answer
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
        };
        let actual = answer
            .get(field)
            .ok_or_else(|| unreadable("the answer has no such value"))?;

        let passed = match (&self.expected, actual) {
            (Expected::Number(op, expected), Value::Number(number)) => {
                op.holds(number.as_f64().unwrap_or(f64::NAN), *expected)
            }
            (Expected::Option(Op::Ne, expected), Value::String(chosen)) => chosen != expected,
            (Expected::Option(_, expected), Value::String(chosen)) => chosen == expected,
            (Expected::AnyOf(options), Value::String(chosen)) => options.contains(chosen),
            _ => return Err(unreadable("the answer is not of the expected type")),
        };
        Ok(Decision {
            condition: self.to_string(),
            passed,
            actual: actual.clone(),
        })
    }
}

fn unknown_option<'a>(
    base: &CliError,
    option: &str,
    known: impl Iterator<Item = &'a String>,
) -> CliError {
    let known: Vec<&str> = known.map(String::as_str).collect();
    let mut error = base.clone();
    error.message =
        format!("`{option}` is not one of the options, so the condition could never hold");
    error.hint(format!("the options are: {}", known.join(", ")))
}

/// The outcome of one condition.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub(crate) struct Decision {
    /// The condition, as it was understood.
    pub(crate) condition: String,
    pub(crate) passed: bool,
    /// The value the condition was compared with.
    pub(crate) actual: Value,
}

/// The outcome of every gate on one evaluation.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub(crate) struct GateOutcome {
    /// `true` when every condition holds. `false` means exit code 10.
    pub(crate) passed: bool,
    /// `true` when the answer fell inside the abstain band, which means exit code 11.
    pub(crate) abstained: bool,
    pub(crate) conditions: Vec<Decision>,
}

impl GateOutcome {
    /// The exit code the outcome stands for. Abstaining wins: "cannot tell" is not "no".
    pub(crate) const fn exit(&self) -> Exit {
        if self.abstained {
            Exit::Abstain
        } else if self.passed {
            Exit::Success
        } else {
            Exit::GateFalse
        }
    }

    /// One line for a person.
    pub(crate) fn describe(&self) -> String {
        let listed: Vec<String> = self
            .conditions
            .iter()
            .map(|decision| {
                format!(
                    "{} ({}, got {})",
                    decision.condition,
                    if decision.passed {
                        "holds"
                    } else {
                        "does not hold"
                    },
                    shown(&decision.actual)
                )
            })
            .collect();
        let verdict = if self.abstained {
            "abstained: the answer is inside the abstain band"
        } else if self.passed {
            "passed"
        } else {
            "failed"
        };
        if listed.is_empty() {
            format!("gate {verdict}")
        } else {
            format!("gate {verdict}: {}", listed.join("; "))
        }
    }
}

fn shown(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

/// A band of probabilities in which a noul is treated as "cannot tell".
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AbstainBand {
    low: f64,
    high: f64,
}

impl AbstainBand {
    /// Parses `lo,hi`, such as `0.4,0.6`.
    ///
    /// # Errors
    ///
    /// A message, ready to show, when the text is not two ordered probabilities.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let malformed = || {
            format!(
                "`{text}` is not a band such as 0.4,0.6: two probabilities from 0 to 1, low first"
            )
        };
        let (low, high) = text.split_once(',').ok_or_else(malformed)?;
        let (low, high) = (
            low.trim().parse::<f64>().map_err(|_| malformed())?,
            high.trim().parse::<f64>().map_err(|_| malformed())?,
        );
        if (0.0..=1.0).contains(&low) && (0.0..=1.0).contains(&high) && low <= high {
            Ok(Self { low, high })
        } else {
            Err(malformed())
        }
    }

    fn contains(self, probability: f64) -> bool {
        (self.low..=self.high).contains(&probability)
    }
}

/// Decides every condition, and the abstain band if there is one.
///
/// Returns `None` when no gate was asked for.
///
/// # Errors
///
/// An internal error (exit 1) when an answer cannot be compared. Never exit 10.
pub(crate) fn decide(
    conditions: &[Condition],
    abstain: Option<(&str, AbstainBand)>,
    answers: &IndexMap<String, Value>,
) -> Result<Option<GateOutcome>, CliError> {
    if conditions.is_empty() && abstain.is_none() {
        return Ok(None);
    }
    let decisions = conditions
        .iter()
        .map(|condition| condition.decide(answers.get(&condition.id)))
        .collect::<Result<Vec<_>, _>>()?;
    let abstained = abstain.is_some_and(|(id, band)| {
        answers
            .get(id)
            .and_then(|answer| answer.get("noul"))
            .and_then(Value::as_f64)
            .is_some_and(|probability| band.contains(probability))
    });
    Ok(Some(GateOutcome {
        passed: decisions.iter().all(|decision| decision.passed),
        abstained,
        conditions: decisions,
    }))
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;
    use jev_client::{Choice, Noul, Request, Score};
    use serde_json::{Value, json};

    use super::{AbstainBand, Condition, Expected, Op, decide};
    use crate::exit::Exit;

    fn answers() -> IndexMap<String, Value> {
        IndexMap::from([
            ("urgent".to_owned(), json!({ "type": "noul", "noul": 0.92 })),
            (
                "team".to_owned(),
                json!({ "type": "choice", "choice": "billing", "confidence": 0.5, "probabilities": {} }),
            ),
            (
                "anger".to_owned(),
                json!({ "type": "score", "score": 1.6, "confidence": 0.78 }),
            ),
            ("odd".to_owned(), json!({ "type": "rank", "order": [] })),
        ])
    }

    fn request() -> Request {
        Request::new("s", "m")
            .question("urgent", Noul::new("?"))
            .question(
                "team",
                Choice::new("?")
                    .option("billing", "a")
                    .option("sales", "b")
                    .bare_option("other"),
            )
            .question("anger", Score::new("?").level("calm").level("angry"))
    }

    #[test]
    fn parses_conditions_with_or_without_spaces() {
        assert_eq!(
            Condition::parse("urgent >= 0.7").unwrap(),
            Condition::number("urgent", false, Op::Ge, 0.7)
        );
        assert_eq!(
            Condition::parse("urgent>=0.7").unwrap(),
            Condition::number("urgent", false, Op::Ge, 0.7)
        );
        assert_eq!(
            Condition::parse(" anger < 1 ").unwrap(),
            Condition::number("anger", false, Op::Lt, 1.0)
        );
        assert_eq!(
            Condition::parse("team.confidence >= 0.8").unwrap(),
            Condition::number("team", true, Op::Ge, 0.8)
        );
        let option = Condition::parse("team != not stated").unwrap();
        assert_eq!(
            option.expected,
            Expected::Option(Op::Ne, "not stated".to_owned())
        );
        assert_eq!(option.to_string(), "team != not stated");
    }

    #[test]
    fn malformed_conditions_are_usage_errors_that_show_the_form() {
        for text in [
            "urgent",
            ">= 0.7",
            "urgent >=",
            "team > billing",
            "team.confidence == high",
            "urgent >= NaN",
        ] {
            let error = Condition::parse(text).unwrap_err();
            assert_eq!(error.exit.code(), 2, "{text}");
            assert!(error.hint.unwrap().contains("is_urgent >= 0.7"), "{text}");
        }
    }

    #[test]
    fn conditions_are_checked_against_the_request_before_anything_is_sent() {
        let request = request();
        for fine in [
            "urgent >= 0.7",
            "team == billing",
            "team.confidence >= 0.8",
            "anger <= 1.5",
            "anger.confidence > 0.5",
        ] {
            assert!(
                Condition::parse(fine)
                    .unwrap()
                    .check_request(&request)
                    .is_ok(),
                "{fine}"
            );
        }
        assert!(
            Condition::any_of("team", vec!["billing".into(), "sales".into()])
                .check_request(&request)
                .is_ok()
        );

        let cases = [
            ("urgnt >= 0.7", "there is no question `urgnt`"),
            ("urgent.confidence >= 0.7", "a noul has no confidence"),
            ("team >= 0.7", "whose answer is an option"),
            ("anger == angry", "answered with a number"),
            ("team == biling", "`biling` is not one of the options"),
        ];
        for (text, message) in cases {
            let error = Condition::parse(text)
                .unwrap()
                .check_request(&request)
                .unwrap_err();
            assert_eq!(error.exit.code(), 2, "{text}");
            assert!(error.message.contains(message), "{text}: {}", error.message);
            assert!(error.hint.is_some(), "{text}");
        }
        let any_of = Condition::any_of("team", vec!["billing".into(), "nope".into()])
            .check_request(&request)
            .unwrap_err();
        assert!(any_of.hint.unwrap().contains("billing, sales, other"));
    }

    #[test]
    fn a_gate_passes_only_when_every_condition_holds() {
        let holds = [
            Condition::parse("urgent >= 0.7").unwrap(),
            Condition::parse("team == billing").unwrap(),
        ];
        let fails = [
            Condition::parse("urgent >= 0.7").unwrap(),
            Condition::parse("team.confidence >= 0.8").unwrap(),
        ];

        let passed = decide(&holds, None, &answers()).unwrap().unwrap();
        let failed = decide(&fails, None, &answers()).unwrap().unwrap();

        assert_eq!((passed.passed, passed.exit()), (true, Exit::Success));
        assert_eq!((failed.passed, failed.exit()), (false, Exit::GateFalse));
        assert_eq!(failed.conditions[1].actual, json!(0.5));
        assert_eq!(
            failed.describe(),
            "gate failed: urgent >= 0.7 (holds, got 0.92); team.confidence >= 0.8 (does not hold, got 0.5)"
        );
        assert_eq!(
            decide(&[], None, &answers()).unwrap(),
            None,
            "no gate, no outcome"
        );
    }

    #[test]
    fn any_of_and_every_operator_behave() {
        let check = |condition: Condition| {
            decide(&[condition], None, &answers())
                .unwrap()
                .unwrap()
                .passed
        };

        assert!(check(Condition::any_of(
            "team",
            vec!["sales".into(), "billing".into()]
        )));
        assert!(!check(Condition::any_of("team", vec!["sales".into()])));
        assert!(check(Condition::parse("team != sales").unwrap()));
        for (text, expected) in [
            ("anger > 1.6", false),
            ("anger >= 1.6", true),
            ("anger < 1.6", false),
            ("anger <= 1.6", true),
            ("anger == 1.6", true),
            ("anger != 1.6", false),
        ] {
            assert_eq!(check(Condition::parse(text).unwrap()), expected, "{text}");
        }
    }

    #[test]
    fn an_answer_that_cannot_be_compared_is_an_error_never_a_false_gate() {
        for condition in [
            Condition::parse("odd >= 0.5").unwrap(),
            Condition::parse("missing >= 0.5").unwrap(),
        ] {
            let error = decide(&[condition], None, &answers()).unwrap_err();

            assert_eq!(
                error.exit.code(),
                1,
                "exit 10 is reserved for a condition that is false"
            );
        }
    }

    #[test]
    fn the_abstain_band_exits_11_and_wins_over_the_gate() {
        let band = AbstainBand::parse("0.4, 0.6").unwrap();
        let mut unsure = answers();
        unsure.insert("urgent".to_owned(), json!({ "type": "noul", "noul": 0.5 }));
        let gate = [Condition::parse("urgent >= 0.7").unwrap()];

        let abstained = decide(&gate, Some(("urgent", band)), &unsure)
            .unwrap()
            .unwrap();
        let confident = decide(&gate, Some(("urgent", band)), &answers())
            .unwrap()
            .unwrap();
        let band_only = decide(&[], Some(("urgent", band)), &unsure)
            .unwrap()
            .unwrap();

        assert_eq!(
            (abstained.abstained, abstained.exit()),
            (true, Exit::Abstain)
        );
        assert_eq!(
            (confident.abstained, confident.exit()),
            (false, Exit::Success)
        );
        assert_eq!(band_only.exit(), Exit::Abstain);
        assert!(abstained.describe().starts_with("gate abstained"));
        for wrong in ["0.6,0.4", "0.4", "a,b", "-0.1,0.5", "0.4,1.5"] {
            assert!(AbstainBand::parse(wrong).is_err(), "{wrong}");
        }
    }
}
