//! CommonMark to `ratatui` spans.
//!
//! An agent writes Markdown, and until now the transcript showed its
//! characters literally rather than their effect — `**bold**` rather than
//! bold, `- item` rather than a bullet. This is the only place that turns
//! `pulldown-cmark`'s events into what a terminal draws.
//!
//! Scope is deliberately narrow: bold, italic, bullets, numbered lists,
//! nesting, code blocks and paragraph breaks — what the plan asked for, plus
//! headings as a free extension of "bold" once a real parser is in hand.
//! Links render their text and drop the URL, inline code loses its
//! backticks but gains no styling, and tables, footnotes, raw HTML and task
//! lists are not modelled: whatever text they still carry passes through
//! unstyled rather than being shown as raw syntax or dropped outright.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// Render one message as lines, under `style` — an entry's own voice (a
/// thought's dim italic, a notice's yellow). Markdown emphasis is layered on
/// top with [`Style::add_modifier`], so `**bold**` inside a thought is still
/// dim, and a notice's `- item` is still yellow.
pub fn render(text: &str, style: Style) -> Vec<Line<'static>> {
    let mut renderer = Renderer::new(style);
    for event in Parser::new_ext(text, Options::empty()) {
        renderer.event(event);
    }
    renderer.finish()
}

/// Walks a `pulldown-cmark` event stream once, building lines as it goes.
struct Renderer {
    /// The entry's own style — everything markdown adds is layered on top.
    base: Style,
    lines: Vec<Line<'static>>,
    /// The line being built. Flushed into `lines` at the end of a block, an
    /// item, or a hard break.
    current: Vec<Span<'static>>,
    /// Nesting depth of `**strong**`/`__strong__`, not a boolean: emphasis
    /// can nest, and the modifier should not lift until the outermost one
    /// closes.
    bold: u32,
    italic: u32,
    /// One entry per open list, `Some(next number)` if ordered, `None` if
    /// bulleted. A stack because lists nest.
    lists: Vec<Option<u64>>,
    /// Whether a block boundary should still add no leading blank line.
    /// Only the very first block in a message gets this treatment — every
    /// other one is preceded by a blank line to separate it visually.
    first_block: bool,
}

impl Renderer {
    fn new(base: Style) -> Self {
        Self {
            base,
            lines: Vec::new(),
            current: Vec::new(),
            bold: 0,
            italic: 0,
            lists: Vec::new(),
            first_block: true,
        }
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            // Inline code loses its backticks but gains no styling — see the
            // module doc's scope note.
            Event::Text(text) | Event::Code(text) => self.text(&text),
            // A soft break is where the author's source happened to wrap; a
            // space keeps the words joined so `Wrap` can re-flow the
            // paragraph to the pane's actual width.
            Event::SoftBreak => self.current.push(Span::raw(" ")),
            // A hard break is a deliberate line break within one block —
            // ends the current line, but is not a new block, so no blank
            // line is inserted.
            Event::HardBreak => self.flush_current(),
            // Links, images, footnotes, raw HTML, math, task-list markers:
            // not modelled. Their inner text (a link's label, for instance)
            // still arrives as its own `Event::Text` and is not lost; only
            // the syntax around it is.
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph | Tag::CodeBlock(_) => self.begin_block(),
            // Rendered bold rather than with a `#` prefix: a natural reading
            // of "bold" once a real parser is doing the work, not a new
            // concept the plan didn't ask for.
            Tag::Heading { .. } => {
                self.begin_block();
                self.bold += 1;
            }
            Tag::List(start) => {
                // A nested list only closes its parent item's own line —
                // inserting a blank line *inside* one list would separate an
                // item from its own children. A top-level list is a block
                // like any other and gets the same blank-line treatment.
                if self.lists.is_empty() {
                    self.begin_block();
                } else {
                    self.flush_current();
                }
                self.lists.push(start);
            }
            Tag::Item => {
                let depth = self.lists.len().saturating_sub(1);
                let marker = match self.lists.last_mut() {
                    Some(Some(number)) => {
                        let marker = format!("{number}. ");
                        *number += 1;
                        marker
                    }
                    // A real bullet, not the dash the model wrote.
                    _ => "\u{2022} ".to_owned(),
                };
                self.current.push(Span::styled(
                    format!("{}{marker}", "  ".repeat(depth)),
                    self.base,
                ));
            }
            Tag::Strong => self.bold += 1,
            Tag::Emphasis => self.italic += 1,
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::CodeBlock | TagEnd::Item => self.flush_current(),
            TagEnd::Heading(_) => {
                self.bold = self.bold.saturating_sub(1);
                self.flush_current();
            }
            TagEnd::List(_) => {
                self.lists.pop();
            }
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            _ => {}
        }
    }

    /// A block starts: a blank line before it, unless it is the first block
    /// in the message — a message never opens with empty space above it.
    fn begin_block(&mut self) {
        if self.first_block {
            self.first_block = false;
        } else {
            self.flush_current();
            self.lines.push(Line::default());
        }
    }

    /// Close the line being built, if anything is on it.
    ///
    /// Guarded on non-empty so a block boundary with nothing pending does
    /// not add a spurious empty `Line` on top of the blank separator
    /// `begin_block` already pushes.
    fn flush_current(&mut self) {
        if !self.current.is_empty() {
            self.lines
                .push(Line::from(std::mem::take(&mut self.current)));
        }
    }

    /// A code block's `Text` events carry embedded `\n`s, one per source
    /// line — a `Span` holding one draws as a single overlong row rather
    /// than several, the exact miscount `panes::paragraphs` was already
    /// written to avoid for the prompt and the transcript's own gutter.
    fn text(&mut self, text: &str) {
        let modifier = (if self.bold > 0 {
            Modifier::BOLD
        } else {
            Modifier::empty()
        }) | (if self.italic > 0 {
            Modifier::ITALIC
        } else {
            Modifier::empty()
        });
        let style = self.base.add_modifier(modifier);

        let mut lines = text.split('\n');
        if let Some(first) = lines.next() {
            self.current.push(Span::styled(first.to_owned(), style));
        }
        for line in lines {
            self.flush_current();
            self.current.push(Span::styled(line.to_owned(), style));
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_current();
        // An empty message would otherwise vanish rather than take the one
        // row the gutter is drawn against.
        if self.lines.is_empty() {
            self.lines.push(Line::default());
        }
        self.lines
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use ratatui::style::Color;

    use super::*;

    /// Every span's text, concatenated line by line — what a reader sees,
    /// independent of how many spans it took to draw it.
    fn text_of(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn plain_text_survives_unchanged() {
        let lines = render("hello world", Style::default());
        assert_eq!(text_of(&lines), ["hello world"]);
    }

    #[test]
    fn bold_text_loses_its_asterisks_but_keeps_the_emphasis() {
        let lines = render("this is **bold** text", Style::default());
        assert_eq!(text_of(&lines), ["this is bold text"]);

        let bold_span = lines[0]
            .spans
            .iter()
            .find(|span| span.content.as_ref() == "bold")
            .expect("a span for the bold word");
        assert!(bold_span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn italic_text_loses_its_asterisks_too() {
        let lines = render("this is *italic* text", Style::default());
        assert_eq!(text_of(&lines), ["this is italic text"]);

        let italic_span = lines[0]
            .spans
            .iter()
            .find(|span| span.content.as_ref() == "italic")
            .expect("a span for the italic word");
        assert!(italic_span.style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn a_bullet_list_item_gets_a_real_bullet_rather_than_a_dash() {
        let lines = render("- one\n- two", Style::default());
        let joined = text_of(&lines).join("\n");

        assert!(joined.contains('\u{2022}'), "{joined}");
        assert!(
            !joined.contains("- "),
            "the source dash leaked through: {joined}"
        );
    }

    #[test]
    fn an_ordered_list_keeps_its_numbers() {
        let lines = render("1. first\n2. second", Style::default());
        let joined = text_of(&lines).join("\n");

        assert!(joined.contains("1. first"), "{joined}");
        assert!(joined.contains("2. second"), "{joined}");
    }

    #[test]
    fn a_nested_list_indents_further_than_its_parent() {
        let lines = render("- parent\n  - child", Style::default());
        let joined = text_of(&lines).join("\n");

        // The child's line starts with more leading space than the parent's.
        let parent_indent = joined
            .lines()
            .find(|line| line.contains("parent"))
            .unwrap()
            .len()
            - joined
                .lines()
                .find(|line| line.contains("parent"))
                .unwrap()
                .trim_start()
                .len();
        let child_indent = joined
            .lines()
            .find(|line| line.contains("child"))
            .unwrap()
            .len()
            - joined
                .lines()
                .find(|line| line.contains("child"))
                .unwrap()
                .trim_start()
                .len();

        assert!(
            child_indent > parent_indent,
            "child ({child_indent}) is not indented further than parent ({parent_indent}): {joined}"
        );
    }

    #[test]
    fn two_paragraphs_are_separated_by_a_blank_line() {
        let lines = render("first paragraph\n\nsecond paragraph", Style::default());
        let texts = text_of(&lines);

        assert_eq!(texts, ["first paragraph", "", "second paragraph"]);
    }

    #[test]
    fn list_items_are_not_separated_by_a_blank_line() {
        let lines = render("- one\n- two", Style::default());
        let texts = text_of(&lines);

        assert!(!texts.contains(&String::new()), "{texts:?}");
    }

    #[test]
    fn a_code_blocks_lines_stay_separate_rather_than_becoming_one_row() {
        // The exact miscount `paragraphs` was already written to avoid,
        // reintroduced here if a code block's embedded newlines ever ended
        // up inside a single `Span`.
        let lines = render("```\nfirst\nsecond\nthird\n```", Style::default());
        let texts = text_of(&lines);

        assert!(texts.iter().any(|line| line == "first"), "{texts:?}");
        assert!(texts.iter().any(|line| line == "second"), "{texts:?}");
        assert!(texts.iter().any(|line| line == "third"), "{texts:?}");
    }

    #[test]
    fn emphasis_inside_a_dim_thought_is_still_dim() {
        let thought_style = Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC);
        let lines = render("the mark needs **more** contrast", thought_style);

        let bold_span = lines[0]
            .spans
            .iter()
            .find(|span| span.content.as_ref() == "more")
            .expect("a span for the emphasised word");

        assert_eq!(bold_span.style.fg, Some(Color::DarkGray));
        assert!(bold_span.style.add_modifier.contains(Modifier::ITALIC));
        assert!(bold_span.style.add_modifier.contains(Modifier::BOLD));
    }
}
