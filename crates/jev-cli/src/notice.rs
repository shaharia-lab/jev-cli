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
    info: bool,
}

impl Notice {
    /// Something that may be wrong, or will be.
    pub(crate) fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
            info: false,
        }
    }

    /// Something that happened and is fine, such as an automatic update: shown without the
    /// `warning:` label, and as `{"info": ...}` for a program.
    pub(crate) fn info(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            info: true,
            ..Self::warning(code, message)
        }
    }

    pub(crate) fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Writes the warning as text for a person, or as one JSON line for a program.
    pub(crate) fn emit(&self, format: Format, ui: Ui, stderr: &mut dyn Write) {
        let text = if format.is_machine_readable() {
            let body = json!({ "code": self.code, "message": self.message, "hint": self.hint });
            let kind = if self.info { "info" } else { "warning" };
            serde_json::Value::Object([(kind.to_owned(), body)].into_iter().collect()).to_string()
        } else {
            let mut text = if self.info {
                self.message.clone()
            } else {
                format!("{} {}", ui.warning_label("warning:"), self.message)
            };
            if let Some(hint) = &self.hint {
                let _ = write!(text, "\n  {} {hint}", ui.dim("hint:"));
            }
            text
        };
        let _ = writeln!(stderr, "{text}");
    }
}

/// Emits notices to stderr the way this run was asked to: as text, as JSON lines, or not at all.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Notifier {
    pub(crate) format: Format,
    pub(crate) ui: Ui,
    pub(crate) quiet: bool,
    /// Whether stderr is a terminal, where something can be redrawn in place.
    pub(crate) terminal: bool,
}

impl Notifier {
    pub(crate) fn emit(self, notice: &Notice) {
        if !self.quiet {
            notice.emit(self.format, self.ui, &mut std::io::stderr().lock());
        }
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

    #[test]
    fn information_has_no_warning_label_and_its_own_json_key() {
        let notice = Notice::info("updated", "jev updated 0.1.0 -> 0.2.0");

        assert_eq!(
            emitted(&notice, Format::Table),
            "jev updated 0.1.0 -> 0.2.0\n"
        );
        assert_eq!(
            emitted(&notice, Format::Jsonl),
            "{\"info\":{\"code\":\"updated\",\"message\":\"jev updated 0.1.0 -> 0.2.0\",\"hint\":null}}\n"
        );
    }
}
