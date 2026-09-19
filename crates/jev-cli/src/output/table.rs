//! Plain aligned columns. No borders: they add noise for people and nothing for anyone else.

use unicode_width::UnicodeWidthStr;

/// Rows of cells, laid out in columns as wide as their widest cell.
///
/// Width is measured in terminal cells, not bytes or characters, so CJK text and accents line up.
/// A cell may carry styling: its visible text is given separately so that escape codes do not
/// count towards the width.
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
    /// A cell whose text is shown as it is.
    pub(crate) fn plain(text: impl Into<String>) -> Self {
        let shown = text.into();
        let width = shown.width();
        Self { shown, width }
    }

    /// A cell showing `styled`, which occupies the same space as `visible`.
    pub(crate) fn styled(visible: &str, styled: String) -> Self {
        Self {
            shown: styled,
            width: visible.width(),
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
    fn an_empty_table_renders_nothing() {
        assert_eq!(Table::default().render(""), "");
    }
}
