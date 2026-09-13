//! Assembling the font database a render draws with.
//!
//! `resvg` implements no `@font-face`, so an SVG cannot carry its own fonts:
//! the only input is the `fontdb::Database` handed to `usvg`. Which fonts are
//! in that database is therefore the difference between a render that is
//! reproducible and one that merely looks right on the machine that made it.
//!
//! The rule here is that system fonts are never loaded speculatively. The
//! document is asked which families it wants, the project's declared font
//! files are loaded, and only if something is still missing does the system
//! get consulted — loudly, and not at all under [`FontPolicy::Strict`].
//!
//! A declared font's bytes are acquired by [`crate::fonts::resolve`], never
//! read directly here — a local file or a checksum-verified, cached fetch
//! are the same one call from this module's point of view, and the one
//! place that call can reach the network (a font declared by URL, on a
//! cache miss) is deliberately not this one. See ADR 015.

use std::collections::BTreeSet;
use std::sync::Arc;

use roxmltree::Document;
use usvg::fontdb::{Database, Family, Query, Source};

use crate::error::{Error, Result};
use crate::project::Project;

/// What to do about a font family the project does not supply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontPolicy {
    /// Fall back to the machine's fonts, and warn that the render is no longer
    /// reproducible elsewhere.
    #[default]
    SystemFallback,
    /// Refuse. What CI should use, so that a missing font is a failed build
    /// rather than a logo silently set in whatever the runner had installed.
    Strict,
}

/// The `font-family` values a document asks for.
///
/// Read from the XML rather than from the resolved render tree because it has
/// to be known *before* the database is built, and because the answer wanted
/// here is what the author wrote, which is also what the diagnostic must
/// quote back at them.
fn required_families(document: &Document<'_>) -> BTreeSet<String> {
    let mut families = BTreeSet::new();

    for node in document.descendants() {
        if let Some(value) = node.attribute("font-family") {
            extend_with_families(&mut families, value);
        }
        // `style="font-family: Inter"` is as common as the presentation
        // attribute, and missing it would mean falling back to system fonts
        // without ever reporting why.
        if let Some(style) = node.attribute("style") {
            for declaration in style.split(';') {
                if let Some((property, value)) = declaration.split_once(':')
                    && property.trim() == "font-family"
                {
                    extend_with_families(&mut families, value);
                }
            }
        }
    }

    families
}

/// Split a `font-family` list into its individual families.
fn extend_with_families(families: &mut BTreeSet<String>, value: &str) {
    for family in value.split(',') {
        let family = family.trim().trim_matches(['"', '\'']).trim();
        // The generics are the CSS keywords every renderer resolves for
        // itself; asking the project to supply a file for `sans-serif` would
        // be nonsense.
        if family.is_empty()
            || matches!(
                family,
                "serif" | "sans-serif" | "cursive" | "fantasy" | "monospace" | "system-ui"
            )
        {
            continue;
        }
        families.insert(family.to_owned());
    }
}

/// Whether the database can already satisfy a family by name.
fn resolves(database: &Database, family: &str) -> bool {
    database
        .query(&Query {
            families: &[Family::Name(family)],
            ..Query::default()
        })
        .is_some()
}

/// Build the font database for a project.
///
/// # Errors
///
/// Returns [`Error::Font`] when a declared font file cannot be read, and
/// [`Error::StrictFonts`] when a family is unresolved under
/// [`FontPolicy::Strict`].
pub fn database(
    project: &Project,
    document: &Document<'_>,
    policy: FontPolicy,
) -> Result<Arc<Database>> {
    let mut database = Database::new();

    for font in &project.metadata().fonts {
        let data = crate::fonts::resolve(project, font)?;
        database.load_font_source(Source::Binary(Arc::new(data)));
    }

    let missing: Vec<String> = required_families(document)
        .into_iter()
        .filter(|family| !resolves(&database, family))
        .collect();

    if !missing.is_empty() {
        match policy {
            FontPolicy::Strict => {
                return Err(Error::StrictFonts {
                    family: missing.join("`, `"),
                });
            }
            FontPolicy::SystemFallback => {
                // Loading the system's fonts is both slow and the thing that
                // makes a render machine-dependent, so it happens here and
                // only here: when the project has already failed to supply
                // something the document asked for by name.
                log::warn!(
                    "font {} not supplied by the project; falling back to system fonts, \
                     so this render may differ on another machine. Declare it with \
                     `<shaipe:font family=\"...\" src=\"...\"/>` to fix it.",
                    missing
                        .iter()
                        .map(|family| format!("`{family}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                database.load_system_fonts();
            }
        }
    }

    Ok(Arc::new(database))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn families(svg: &str) -> Vec<String> {
        let document = Document::parse(svg).unwrap();
        required_families(&document).into_iter().collect()
    }

    #[test]
    fn font_families_are_collected_from_attributes_and_from_inline_styles() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg">
            <text font-family="Inter">a</text>
            <text style="fill:red; font-family: 'Fira Code', monospace">b</text>
        </svg>"#;
        assert_eq!(families(svg), ["Fira Code", "Inter"]);
    }

    #[test]
    fn generic_css_families_are_not_treated_as_missing_fonts() {
        // Warning that the project failed to supply `sans-serif` would be
        // noise on every document that sets a sensible fallback chain.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg">
            <text font-family="sans-serif, monospace, system-ui">a</text>
        </svg>"#;
        assert!(families(svg).is_empty());
    }

    #[test]
    fn a_document_with_no_text_requires_no_fonts_at_all() {
        // The common case for a logo, and the reason system fonts are never
        // loaded speculatively: it would cost every render for nothing.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="1" height="1"/></svg>"#;
        assert!(families(svg).is_empty());
    }
}
