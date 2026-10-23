//! Selecting text on screen, and copying it.
//!
//! The workspace captures the mouse so that panes can be clicked and dividers
//! dragged, and a terminal that has handed its mouse over has no selection of
//! its own any more. This is the replacement: a drag inside a pane highlights
//! what is under it, and letting go copies it.
//!
//! It works on the *drawn frame* rather than on each pane's contents. A
//! selection that read the transcript's lines would need an implementation per
//! pane, and each would disagree with the screen about where a wrapped line
//! breaks, which is the one thing a person dragging across text is looking at.
//! Reading cells means that whatever is on the screen can be copied, and that
//! what is copied is what was highlighted. The price is that a soft-wrapped
//! paragraph copies as the rows it was drawn in.

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use super::app::App;

/// Text being, or just done being, selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// Where the drag started and may roam: a pane's interior.
    ///
    /// Bounded, because the panes sit side by side and a terminal-style
    /// selection runs along rows — across the border and into the next column,
    /// which is never what anybody meant to copy.
    region: Rect,
    anchor: Position,
    head: Position,
    /// Whether the button has been released.
    finished: bool,
    /// Whether the text has already gone to the clipboard.
    copied: bool,
}

impl Selection {
    /// Start a selection at a point, which is taken to be inside `region`.
    #[must_use]
    pub fn begin(region: Rect, column: u16, row: u16) -> Self {
        let at = Self::clamp(region, column, row);
        Self {
            region,
            anchor: at,
            head: at,
            finished: false,
            copied: false,
        }
    }

    /// `(column, row)` brought inside `region`.
    fn clamp(region: Rect, column: u16, row: u16) -> Position {
        // A zero-sized region has no last column to clamp to.
        let last_column = region.right().saturating_sub(1).max(region.x);
        let last_row = region.bottom().saturating_sub(1).max(region.y);
        Position::new(
            column.clamp(region.x, last_column),
            row.clamp(region.y, last_row),
        )
    }

    /// Move the loose end, wherever the pointer has got to.
    pub fn extend(&mut self, column: u16, row: u16) {
        if !self.finished {
            self.head = Self::clamp(self.region, column, row);
        }
    }

    /// The button went up. Whether anything was selected, which a plain click
    /// is not.
    pub fn finish(&mut self) -> bool {
        self.finished = true;
        self.anchor != self.head
    }

    /// Whether this was only a click, which selects nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// Whether the button has been released and the text not yet taken.
    #[must_use]
    pub const fn awaiting_copy(&self) -> bool {
        self.finished && !self.copied
    }

    /// Note that the text has been taken, so a second frame does not take it
    /// again.
    pub const fn mark_copied(&mut self) {
        self.copied = true;
    }

    /// The two ends in reading order: first the one nearer the top, and on the
    /// same row the one nearer the left.
    fn ordered(&self) -> (Position, Position) {
        if (self.anchor.y, self.anchor.x) <= (self.head.y, self.head.x) {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// The columns of `row` that are selected, as a half-open range.
    fn columns_in(&self, row: u16) -> Option<(u16, u16)> {
        let (start, end) = self.ordered();
        if row < start.y || row > end.y {
            return None;
        }
        // Rows between the ends are taken whole, the first from the anchor to
        // the edge and the last from the edge to the head: the way every
        // terminal selects, and the way a paragraph reads.
        let from = if row == start.y {
            start.x
        } else {
            self.region.x
        };
        let to = if row == end.y {
            end.x.saturating_add(1)
        } else {
            self.region.right()
        };
        Some((from, to.min(self.region.right())))
    }

    /// Draw the selection into the frame, reversing what it covers.
    pub fn highlight(&self, buffer: &mut Buffer) {
        for row in self.region.top()..self.region.bottom() {
            let Some((from, to)) = self.columns_in(row) else {
                continue;
            };
            for column in from..to {
                if let Some(cell) = buffer.cell_mut((column, row)) {
                    cell.set_style(Style::default().add_modifier(Modifier::REVERSED));
                }
            }
        }
    }

    /// What is under the selection, as it is drawn in `buffer`.
    #[must_use]
    pub fn text(&self, buffer: &Buffer) -> String {
        let mut rows = Vec::new();

        for row in self.region.top()..self.region.bottom() {
            let Some((from, to)) = self.columns_in(row) else {
                continue;
            };

            let mut line = String::new();
            let mut column = from;
            while column < to {
                let Some(cell) = buffer.cell((column, row)) else {
                    break;
                };
                let symbol = cell.symbol();
                line.push_str(symbol);
                // A wide glyph occupies the cell after it as well, and that
                // one reads as a space. Stepping over it is what stops "日本"
                // copying as "日 本 ".
                column = column
                    .saturating_add(u16::try_from(Span::raw(symbol).width()).unwrap_or(1).max(1));
            }

            // The padding to the edge of the pane is not text, and a copied
            // paragraph that ends every row in forty spaces is a mess to paste.
            rows.push(line.trim_end().to_owned());
        }

        rows.join("\n").trim_end().to_owned()
    }
}

/// Draw the workspace's selection over a finished frame, and take its text the
/// first time the frame shows it released.
///
/// Last, after every widget and any modal, because it reads and restyles the
/// cells those left — and from the frame rather than from the state, because
/// the frame is what the person dragged across.
pub fn paint(buffer: &mut Buffer, app: &mut App) {
    let Some(selection) = app.text_selection().cloned() else {
        return;
    };

    selection.highlight(buffer);

    if selection.awaiting_copy() {
        // Marked first, so the second frame an arrival can cost does not
        // copy it again.
        app.mark_selection_copied();
        app.copy_to_clipboard(selection.text(buffer));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A buffer holding `rows`, one per line, padded to the width of the
    /// widest.
    fn drawn(rows: &[&str]) -> Buffer {
        // Columns, not characters: a wide glyph is one of the second and two of
        // the first.
        let width = rows
            .iter()
            .map(|row| Span::raw(*row).width())
            .max()
            .unwrap_or(0);
        let mut buffer = Buffer::empty(Rect::new(
            0,
            0,
            u16::try_from(width).unwrap(),
            u16::try_from(rows.len()).unwrap(),
        ));
        for (y, row) in rows.iter().enumerate() {
            buffer.set_string(0, u16::try_from(y).unwrap(), row, Style::default());
        }
        buffer
    }

    fn selected(buffer: &Buffer, from: (u16, u16), to: (u16, u16)) -> Selection {
        let mut selection = Selection::begin(buffer.area, from.0, from.1);
        selection.extend(to.0, to.1);
        selection.finish();
        selection
    }

    #[test]
    fn a_selection_within_one_row_copies_only_the_cells_it_covers() {
        let buffer = drawn(&["hello world"]);

        assert_eq!(selected(&buffer, (6, 0), (10, 0)).text(&buffer), "world");
    }

    #[test]
    fn a_selection_dragged_backwards_copies_the_same_text() {
        let buffer = drawn(&["hello world"]);

        assert_eq!(selected(&buffer, (10, 0), (6, 0)).text(&buffer), "world");
    }

    #[test]
    fn a_selection_over_several_rows_takes_the_middle_ones_whole() {
        let buffer = drawn(&["first line", "second line", "third line"]);

        assert_eq!(
            selected(&buffer, (6, 0), (4, 2)).text(&buffer),
            "line\nsecond line\nthird"
        );
    }

    #[test]
    fn padding_at_the_end_of_a_row_is_not_copied() {
        let buffer = drawn(&["short", "a much longer row"]);

        assert_eq!(
            selected(&buffer, (0, 0), (16, 1)).text(&buffer),
            "short\na much longer row"
        );
    }

    #[test]
    fn a_drag_past_the_pane_stays_inside_it() {
        let buffer = drawn(&["0123456789", "abcdefghij"]);
        let region = Rect::new(2, 0, 4, 2);

        let mut selection = Selection::begin(region, 3, 0);
        selection.extend(40, 40);
        selection.finish();

        assert_eq!(selection.text(&buffer), "345\ncdef");
    }

    #[test]
    fn a_click_without_a_drag_selects_nothing() {
        let buffer = drawn(&["hello"]);
        let mut selection = Selection::begin(buffer.area, 2, 0);

        assert!(!selection.finish());
        assert!(selection.is_empty());
    }

    #[test]
    fn a_wide_glyph_is_copied_once_and_not_followed_by_a_space() {
        let buffer = drawn(&["日本 ok"]);

        assert_eq!(selected(&buffer, (0, 0), (6, 0)).text(&buffer), "日本 ok");
    }

    #[test]
    fn highlighting_reverses_exactly_the_selected_cells() {
        let mut buffer = drawn(&["abcdef"]);
        selected(&buffer.clone(), (1, 0), (3, 0)).highlight(&mut buffer);

        let reversed: Vec<bool> = (0..6)
            .map(|column| {
                buffer
                    .cell((column, 0))
                    .unwrap()
                    .modifier
                    .contains(Modifier::REVERSED)
            })
            .collect();
        assert_eq!(reversed, [false, true, true, true, false, false]);
    }

    #[test]
    fn the_text_is_taken_once() {
        let buffer = drawn(&["hello"]);
        let mut selection = selected(&buffer, (0, 0), (4, 0));

        assert!(selection.awaiting_copy());
        selection.mark_copied();
        assert!(!selection.awaiting_copy());
    }
}
