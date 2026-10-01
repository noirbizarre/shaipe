//! The text an SVG sets, as its author declared it.
//!
//! Shaipe cannot read lettering out of a raster (ADR 004) so the reading is
//! the agent's, and it is written into the artwork as an ordinary `<text>`
//! element. That is the only place it can live without a second source of
//! truth (ADR 001), so this is where it is read back: what the text says,
//! which family it asks for, and how sure its author said they were.
//!
//! Certainty is an SVG convention, not a project format: two `data-*`
//! attributes on the `<text>` element, which `usvg` ignores and every browser
//! keeps. They never touch geometry, so the file stays an ordinary SVG
//! (invariant 10), and `saving_an_untouched_project_would_not_change_a_byte`
//! has nothing to learn about them. See ADR 028.

use roxmltree::{Document, Node};
use serde::Serialize;

/// The attribute that says how sure the author is of what the text reads and
/// of its font: `high`, `medium` or `low`.
pub const CONFIDENCE_ATTRIBUTE: &str = "data-shaipe-confidence";
/// The attribute that says what is doubtful, in the author's words.
pub const NOTE_ATTRIBUTE: &str = "data-shaipe-note";

/// One `<text>` element of an SVG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeclaredText {
    /// What it says: its text content with runs of whitespace collapsed,
    /// `<tspan>` children included.
    pub content: String,
    /// The family it asks for, from the element or the nearest ancestor that
    /// sets one, the way the renderer inherits it. The first of a list.
    pub font_family: Option<String>,
    /// The `data-shaipe-confidence` it carries, as written.
    pub confidence: Option<String>,
    /// The `data-shaipe-note` it carries.
    pub note: Option<String>,
}

/// Every `<text>` element of `svg`, in document order.
///
/// A document that does not parse has none: the caller has rendered it, so it
/// parsed once already, and a failure here is not worth refusing a comparison
/// for.
#[must_use]
pub fn declared_text(svg: &str) -> Vec<DeclaredText> {
    let Ok(document) = Document::parse(svg) else {
        return Vec::new();
    };
    document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "text")
        .map(|node| DeclaredText {
            content: node
                .descendants()
                .filter(Node::is_text)
                .filter_map(|text| text.text())
                .collect::<Vec<_>>()
                .join("")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            font_family: inherited_family(node),
            confidence: node.attribute(CONFIDENCE_ATTRIBUTE).map(str::to_owned),
            note: node.attribute(NOTE_ATTRIBUTE).map(str::to_owned),
        })
        .collect()
}

/// The first family of the nearest `font-family` up the tree. Generic
/// families count: they are what the element asks for, though no file backs
/// them.
fn inherited_family(node: Node<'_, '_>) -> Option<String> {
    node.ancestors().find_map(|ancestor| {
        let value = ancestor.attribute("font-family").or_else(|| {
            // `style="font-family: Inter"` is as common as the attribute.
            ancestor.attribute("style")?.split(';').find_map(|rule| {
                let (property, value) = rule.split_once(':')?;
                (property.trim() == "font-family").then_some(value)
            })
        })?;
        value
            .split(',')
            .next()
            .map(|family| family.trim().trim_matches(['"', '\'']).trim().to_owned())
            .filter(|family| !family.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_read_with_its_spans_joined_and_whitespace_collapsed() {
        let found = declared_text(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="1">Aca <tspan font-weight="700">me</tspan>
              Studio</text></svg>"#,
        );

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].content, "Aca me Studio");
    }

    #[test]
    fn the_family_is_inherited_from_the_nearest_ancestor_that_sets_one() {
        let found = declared_text(
            r#"<svg xmlns="http://www.w3.org/2000/svg" font-family="Serif One">
                 <g font-family="'Fira Sans', sans-serif"><text>A</text></g>
                 <g style="fill: red; font-family: Inter"><text>B</text></g>
                 <text font-family="Own">C</text>
                 <text>D</text>
               </svg>"#,
        );
        let families: Vec<_> = found.iter().map(|t| t.font_family.as_deref()).collect();

        assert_eq!(
            families,
            [
                Some("Fira Sans"),
                Some("Inter"),
                Some("Own"),
                Some("Serif One")
            ]
        );
    }

    #[test]
    fn the_authors_certainty_and_doubt_are_read_back() {
        let found = declared_text(&format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text {CONFIDENCE_ATTRIBUTE}="low" {NOTE_ATTRIBUTE}="second letter is a or o">Acme</text><text>Sure</text></svg>"#
        ));

        assert_eq!(found[0].confidence.as_deref(), Some("low"));
        assert_eq!(found[0].note.as_deref(), Some("second letter is a or o"));
        assert_eq!(found[1].confidence, None);
    }

    #[test]
    fn a_document_that_does_not_parse_has_no_text() {
        assert!(declared_text("<svg><text>").is_empty());
    }
}
