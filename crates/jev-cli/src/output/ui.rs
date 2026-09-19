//! How human output looks: colour and character set.

use anstyle::{AnsiColor, Style};

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

    fn paint(self, style: Style, text: &str) -> String {
        if self.color {
            format!("{style}{text}{style:#}")
        } else {
            text.to_owned()
        }
    }

    pub(crate) fn bold(self, text: &str) -> String {
        self.paint(Style::new().bold(), text)
    }

    pub(crate) fn dim(self, text: &str) -> String {
        self.paint(Style::new().dimmed(), text)
    }

    // Drawn by the result envelope, which `jev eval` (issue #9) is the first command to print.
    #[cfg_attr(not(feature = "internal-test-hooks"), allow(dead_code))]
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
    // Drawn by the result envelope, which `jev eval` (issue #9) is the first command to print.
    #[cfg_attr(not(feature = "internal-test-hooks"), allow(dead_code))]
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
    #[cfg(test)]
    pub(crate) const fn has_unicode(self) -> bool {
        self.unicode
    }

    /// The separator between facts on one line.
    // Drawn by the result envelope, which `jev eval` (issue #9) is the first command to print.
    #[cfg_attr(not(feature = "internal-test-hooks"), allow(dead_code))]
    pub(crate) const fn separator(self) -> &'static str {
        if self.unicode { " · " } else { " | " }
    }
}

#[cfg(test)]
mod tests {
    use super::Ui;

    #[test]
    fn plain_output_has_no_escape_codes_and_no_unicode() {
        let ui = Ui::plain();

        assert_eq!(ui.bold("x"), "x");
        assert_eq!(ui.error_label("error:"), "error:");
        assert_eq!(ui.bar(0.5, 10), "#####.....");
        assert_eq!(ui.separator(), " | ");
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
