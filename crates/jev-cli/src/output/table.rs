//! Plain aligned columns. No borders: they add noise for people and nothing for anyone else.

use std::borrow::Cow;

use unicode_width::UnicodeWidthStr;

use super::ui::printable;

/// Rows of cells, laid out in columns as wide as their widest cell.
///
/// Width is measured in terminal cells, not bytes or characters, so CJK text and accents line up.
/// A cell may carry styling: its visible text is given separately so that escape codes do not
/// count towards the width.
///
/// A cell holds a value, which may come from a server or a file, so its text goes through
/// [`printable`]: a control character in it can neither drive the terminal nor throw the columns
/// out by being counted as zero cells wide.
#[derive(Debug, Default)]
pub(crate) struct Table {
    rows: Vec<Vec<Cell>>,
}

#[derive(Debug)]
pub(crate) struct Cell {
    shown: String,
    width: usize,
}

impl Cell {
    /// A cell whose text is shown as it is, bar control characters.
    pub(crate) fn plain(text: impl Into<String>) -> Self {
        let text = text.into();
        let shown = match printable(&text) {
            Cow::Borrowed(_) => text,
            Cow::Owned(escaped) => escaped,
        };
        let width = shown.width();
        Self { shown, width }
    }

    /// A cell showing `styled`, which occupies the same space as `visible`.
    ///
    /// `styled` comes from [`Ui`](super::Ui), which has already neutralised `visible`'s control
    /// characters, so the width is measured on the same text the terminal will see.
    pub(crate) fn styled(visible: &str, styled: String) -> Self {
        Self {
            shown: styled,
            width: printable(visible).width(),
        }
    }
}

impl Table {
    pub(crate) fn row(&mut self, cells: Vec<Cell>) {
        self.rows.push(cells);
    }

    /// Lays the rows out, each line prefixed by `indent`. The last column is never padded, so no
    /// line ends in spaces.
    pub(crate) fn render(&self, indent: &str) -> String {
        let columns = self.rows.iter().map(Vec::len).max().unwrap_or(0);
        let widths: Vec<usize> = (0..columns)
            .map(|column| {
                self.rows
                    .iter()
                    .filter_map(|row| row.get(column))
                    .map(|cell| cell.width)
                    .max()
                    .unwrap_or(0)
            })
            .collect();

        let mut text = String::new();
        for row in &self.rows {
            text.push_str(indent);
            for (index, (cell, width)) in row.iter().zip(&widths).enumerate() {
                text.push_str(&cell.shown);
                if index + 1 < row.len() {
                    text.extend(std::iter::repeat_n(' ', width - cell.width + 2));
                }
            }
            text.push('\n');
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::{Cell, Table};

    #[test]
    fn columns_are_as_wide_as_their_widest_cell_and_lines_do_not_end_in_spaces() {
        let mut table = Table::default();
        table.row(vec![Cell::plain("NAME"), Cell::plain("RELEASED")]);
        table.row(vec![Cell::plain("jev-latest"), Cell::plain("2026-09-10")]);
        table.row(vec![Cell::plain("jev")]);

        assert_eq!(
            table.render("  "),
            "  NAME        RELEASED\n  jev-latest  2026-09-10\n  jev\n"
        );
    }

    #[test]
    fn width_is_measured_in_terminal_cells() {
        let mut table = Table::default();
        table.row(vec![Cell::plain("支払い"), Cell::plain("x")]);
        table.row(vec![Cell::plain("abcdef"), Cell::plain("y")]);
        table.row(vec![
            Cell::styled("ab", "\u{1b}[1mab\u{1b}[0m".to_owned()),
            Cell::plain("z"),
        ]);

        assert_eq!(
            table.render(""),
            "支払い  x\nabcdef  y\n\u{1b}[1mab\u{1b}[0m      z\n"
        );
    }

    #[test]
    fn a_control_character_in_a_value_neither_drives_the_terminal_nor_skews_the_columns() {
        let mut table = Table::default();
        // As a server or a request file could send them: a hidden-text sequence and a carriage
        // return, which would otherwise overwrite the line to its left.
        table.row(vec![
            Cell::plain("a\u{1b}[8mb"),
            Cell::styled("a\rb", "a\\u{d}b".to_owned()),
            Cell::plain("end"),
        ]);
        // Both cells above are 11 and 7 cells wide once escaped, so neither column moves.
        table.row(vec![
            Cell::plain("0123456789A"),
            Cell::plain("0123456"),
            Cell::plain("end"),
        ]);

        let rendered = table.render("");

        assert!(
            !rendered.contains('\u{1b}') && !rendered.contains('\r'),
            "{rendered:?}"
        );
        assert_eq!(
            rendered,
            "a\\u{1b}[8mb  a\\u{d}b  end\n0123456789A  0123456  end\n"
        );
    }

    #[test]
    fn an_empty_table_renders_nothing() {
        assert_eq!(Table::default().render(""), "");
    }
}
