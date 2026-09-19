//! The one place that decides whether `jev` may ask a question.
//!
//! An agent, a script or a CI job cannot answer a prompt, and a tool that waits for one hangs
//! forever. So a prompt is only ever shown to a person at a terminal, and everywhere else the
//! would-be prompt becomes an error that names the flag or environment variable to use instead.

use crate::error::CliError;

/// Whether prompting is possible, and if not, why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Interaction {
    stdin_is_terminal: bool,
    no_input: bool,
    ci: bool,
}

impl Interaction {
    /// `ci` is the value of the `CI` environment variable, if it is set.
    pub(crate) fn new(stdin_is_terminal: bool, no_input: bool, ci: Option<&str>) -> Self {
        let ci = ci.is_some_and(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "" | "0" | "false" | "no"
            )
        });
        Self {
            stdin_is_terminal,
            no_input,
            ci,
        }
    }

    /// Succeeds only when a prompt can be shown and answered.
    ///
    /// `what` is the thing that would be asked for ("the API key") and `instead` tells the reader
    /// how to supply it without a prompt ("pipe it to `jev auth login --with-token`").
    ///
    /// # Errors
    ///
    /// A usage error saying why a prompt is impossible and what to do instead.
    // First used outside the test hooks by `jev auth login` (issue #10).
    #[cfg_attr(not(feature = "internal-test-hooks"), allow(dead_code))]
    pub(crate) fn ensure_can_prompt(self, what: &str, instead: &str) -> Result<(), CliError> {
        let reason = if self.no_input {
            "--no-input (or JEV_NO_INPUT) is set"
        } else if self.ci {
            "CI is set, so nobody is there to answer"
        } else if !self.stdin_is_terminal {
            "stdin is not a terminal"
        } else {
            return Ok(());
        };
        Err(CliError::prompt_not_allowed(what, reason, instead))
    }
}

#[cfg(test)]
mod tests {
    use super::Interaction;

    #[test]
    fn a_person_at_a_terminal_can_be_asked() {
        assert!(
            Interaction::new(true, false, None)
                .ensure_can_prompt("the API key", "use --with-token")
                .is_ok()
        );
        assert!(
            Interaction::new(true, false, Some("false"))
                .ensure_can_prompt("x", "y")
                .is_ok()
        );
        assert!(
            Interaction::new(true, false, Some("0"))
                .ensure_can_prompt("x", "y")
                .is_ok()
        );
    }

    #[test]
    fn everyone_else_gets_an_error_that_says_what_to_do() {
        let cases = [
            (
                Interaction::new(false, false, None),
                "stdin is not a terminal",
            ),
            (
                Interaction::new(true, true, None),
                "--no-input (or JEV_NO_INPUT) is set",
            ),
            (
                Interaction::new(true, false, Some("true")),
                "CI is set, so nobody is there to answer",
            ),
            (
                Interaction::new(true, false, Some("1")),
                "CI is set, so nobody is there to answer",
            ),
        ];

        for (interaction, reason) in cases {
            let error = interaction
                .ensure_can_prompt("the API key", "pipe it to `jev auth login --with-token`")
                .unwrap_err();

            assert_eq!(error.exit.code(), 2);
            assert_eq!(error.code, "input_required");
            assert_eq!(
                error.message,
                format!("cannot ask for the API key: {reason}")
            );
            assert_eq!(
                error.hint.as_deref(),
                Some("pipe it to `jev auth login --with-token`")
            );
        }
    }
}
