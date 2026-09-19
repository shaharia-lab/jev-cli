//! How human output looks: colour and character set.

use std::borrow::Cow;

use anstyle::{AnsiColor, Style};

/// `text` with every control character except newline and tab written out as an escape, such as
/// `\u{1b}`, so that it cannot drive a terminal.
///
/// For text that `jev` did not write itself and prints for a person: a server's error message or
/// a value from a file. A terminal escape in it could otherwise retitle the window, rewrite what
/// is already on screen, or hide a line.
pub(crate) fn printable(text: &str) -> Cow<'_, str> {
    let is_unsafe = |c: char| c.is_control() && c != '\n' && c != '\t';
    if !text.contains(is_unsafe) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        text.chars()
            .map(|c| {
                if is_unsafe(c) {
                    c.escape_unicode().to_string()
                } else {
                    c.to_string()
                }
            })
            .collect(),
    )
}

/// The look of human-readable output for one stream.
///
/// Colour is off unless the stream is a terminal, `NO_COLOR` is unset and `--no-color` was not
/// passed. Unicode is off when `--ascii` was passed or the locale is not UTF-8, so output stays
/// legible on any terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ui {
    color: bool,
    unicode: bool,
}

impl Ui {
    pub(crate) const fn new(color: bool, unicode: bool) -> Self {
        Self { color, unicode }
    }

    /// No colour, ASCII only: what a pipe, a log file or a dumb terminal gets.
    #[cfg(test)]
    pub(crate) const fn plain() -> Self {
        Self::new(false, false)
    }

    /// Styles one value for a person.
    ///
    /// `text` is always a value, never a line that has already been styled, so it is passed
    /// through [`printable`] first: whatever `jev` paints for a person cannot drive the terminal,
    /// whether colour is on or off. The escape codes added here are `jev`'s own and stay.
    fn paint(self, style: Style, text: &str) -> String {
        let text = printable(text);
        if self.color {
            format!("{style}{text}{style:#}")
        } else {
            text.into_owned()
        }
    }

    pub(crate) fn bold(self, text: &str) -> String {
        self.paint(Style::new().bold(), text)
    }

    pub(crate) fn dim(self, text: &str) -> String {
        self.paint(Style::new().dimmed(), text)
    }

    pub(crate) fn accent(self, text: &str) -> String {
        self.paint(Style::new().fg_color(Some(AnsiColor::Cyan.into())), text)
    }

    pub(crate) fn warning_label(self, text: &str) -> String {
        self.paint(
            Style::new().bold().fg_color(Some(AnsiColor::Yellow.into())),
            text,
        )
    }

    pub(crate) fn error_label(self, text: &str) -> String {
        self.paint(
            Style::new().bold().fg_color(Some(AnsiColor::Red.into())),
            text,
        )
    }

    /// A horizontal bar of `width` cells, filled in proportion to `fraction` (0 to 1).
    pub(crate) fn bar(self, fraction: f64, width: usize) -> String {
        let (full, empty) = if self.unicode {
            ('█', '░')
        } else {
            ('#', '.')
        };
        let fraction = if fraction.is_nan() {
            0.0
        } else {
            fraction.clamp(0.0, 1.0)
        };
        // `fraction` is within 0 to 1 and `width` is a handful of cells, so the cast is exact.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let filled = ((fraction * width as f64).round() as usize).min(width);
        let mut bar: String = std::iter::repeat_n(full, filled).collect();
        bar.extend(std::iter::repeat_n(empty, width - filled));
        if self.color { self.accent(&bar) } else { bar }
    }

    /// Whether colour is on.
    pub(crate) const fn has_color(self) -> bool {
        self.color
    }

    /// Whether Unicode characters are used.
    pub(crate) const fn has_unicode(self) -> bool {
        self.unicode
    }

    /// The separator between facts on one line.
    pub(crate) const fn separator(self) -> &'static str {
        if self.unicode { " · " } else { " | " }
    }
}

#[cfg(test)]
mod tests {
    use super::{Ui, printable};

    #[test]
    fn printable_text_cannot_drive_a_terminal_but_keeps_its_lines() {
        assert_eq!(printable("plain\n\ttext"), "plain\n\ttext");
        assert_eq!(
            printable("bad \u{1b}]0;title\u{7}\u{1b}[2J\r\u{9b}x"),
            "bad \\u{1b}]0;title\\u{7}\\u{1b}[2J\\u{d}\\u{9b}x"
        );
    }

    #[test]
    fn plain_output_has_no_escape_codes_and_no_unicode() {
        let ui = Ui::plain();

        assert_eq!(ui.bold("x"), "x");
        assert_eq!(ui.error_label("error:"), "error:");
        assert_eq!(ui.bar(0.5, 10), "#####.....");
        assert_eq!(ui.separator(), " | ");
    }

    #[test]
    fn styling_a_value_neutralises_its_control_characters_with_colour_on_or_off() {
        for ui in [Ui::plain(), Ui::new(true, true)] {
            for styled in [ui.bold("a\u{1b}[8mb"), ui.dim("a\rb")] {
                assert!(
                    !styled.contains("\u{1b}[8m") && !styled.contains('\r'),
                    "{styled:?}"
                );
            }
        }
        assert_eq!(Ui::plain().bold("a\u{1b}[8mb"), "a\\u{1b}[8mb");
    }

    #[test]
    fn colour_wraps_text_in_escape_codes_that_are_reset() {
        let painted = Ui::new(true, true).error_label("error:");

        assert!(painted.starts_with("\u{1b}["), "{painted:?}");
        assert!(painted.ends_with("\u{1b}[0m"), "{painted:?}");
        assert!(painted.contains("error:"));
    }

    #[test]
    fn bars_are_always_the_requested_width() {
        let ui = Ui::new(false, true);

        assert_eq!(ui.bar(0.0, 4), "░░░░");
        assert_eq!(ui.bar(1.0, 4), "████");
        assert_eq!(ui.bar(0.26, 4), "█░░░");
        for odd in [-1.0, 7.0, f64::NAN, f64::INFINITY] {
            assert_eq!(ui.bar(odd, 4).chars().count(), 4, "{odd}");
        }
    }
}
