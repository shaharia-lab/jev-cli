//! Warnings and notices: things worth saying that are not the command's result and not a failure.
//!
//! They go to stderr, so stdout stays data. When the output is for a program every line on stderr
//! is a JSON object, a warning as much as the final error, so a caller parsing stderr never meets
//! stray text. `--quiet` suppresses them.

use std::fmt::Write as _;
use std::io::Write;

use serde_json::json;

use crate::output::{Format, Ui};

/// Something the user should know, with a stable code and, when there is one, a next action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Notice {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) hint: Option<String>,
}

impl Notice {
    pub(crate) fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
        }
    }

    pub(crate) fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Writes the warning as text for a person, or as one JSON line for a program.
    pub(crate) fn emit(&self, format: Format, ui: Ui, stderr: &mut dyn Write) {
        let text = if format.is_machine_readable() {
            json!({ "warning": { "code": self.code, "message": self.message, "hint": self.hint } })
                .to_string()
        } else {
            let mut text = format!("{} {}", ui.warning_label("warning:"), self.message);
            if let Some(hint) = &self.hint {
                let _ = write!(text, "\n  {} {hint}", ui.dim("hint:"));
            }
            text
        };
        let _ = writeln!(stderr, "{text}");
    }
}

#[cfg(test)]
mod tests {
    use super::Notice;
    use crate::output::{Format, Ui};

    fn emitted(notice: &Notice, format: Format) -> String {
        let mut stderr = Vec::new();
        notice.emit(format, Ui::plain(), &mut stderr);
        String::from_utf8(stderr).unwrap()
    }

    #[test]
    fn a_person_gets_text_and_a_program_gets_one_json_line() {
        let notice = Notice::warning("config_unknown_key", "unknown key `modle`")
            .hint("did you mean `model`?");

        assert_eq!(
            emitted(&notice, Format::Table),
            "warning: unknown key `modle`\n  hint: did you mean `model`?\n"
        );
        assert_eq!(
            emitted(&notice, Format::Json),
            "{\"warning\":{\"code\":\"config_unknown_key\",\"message\":\"unknown key `modle`\",\"hint\":\"did you mean `model`?\"}}\n"
        );
    }
}
