//! Isolating one variant of a project as a standalone SVG document.
//!
//! # Why this exists
//!
//! The obvious way to render one part of a document is
//! [`usvg::Tree::node_by_id`] followed by [`resvg::render_node`]. It does not
//! work here: `usvg` resolves `<use>` by inlining it and then discards the
//! `<symbol>` definitions entirely, so a variant's element is not in the
//! render tree to be found. This was checked, not assumed.
//!
//! So the variant is isolated at the XML level instead, before `usvg` ever
//! sees it. The document's definitions are kept, its root-level *drawn*
//! content is dropped, and a single `<use>` of the variant is put in its
//! place. The root then carries the variant's own `viewBox` and the
//! specification's pixel size, which lets SVG's own `preserveAspectRatio` do
//! the fitting — uniformly scaled and centred, with no arithmetic here to get
//! wrong.
//!
//! It also means `Format::Svg` is not a separate feature: an isolated variant
//! *is* a standalone SVG document.

use roxmltree::{Document, Node};

use crate::error::Result;
use crate::project::spec::RenderSpec;
use crate::project::{Project, SVG_NAMESPACE};

/// The SVG elements that draw something.
///
/// A denylist rather than an allowlist because *this* set is the closed one:
/// SVG's drawable elements are enumerated by the specification, while the
/// things that merely define — gradients, filters, patterns, clip paths,
/// masks, markers, symbols — are what everything else is. Keeping an element
/// that turned out to be a definition is harmless; dropping one would break
/// every fill that referenced it.
const DRAWN_ELEMENTS: &[&str] = &[
    "a",
    "circle",
    "ellipse",
    "foreignObject",
    "g",
    "image",
    "line",
    "path",
    "polygon",
    "polyline",
    "rect",
    "svg",
    "switch",
    "text",
    "use",
];

/// Elements that describe the *project* rather than the artwork.
///
/// Dropped on isolation because an isolated variant is a derived asset, not a
/// project. Carrying `<metadata>` through would stamp the prompt, the palette
/// and every render specification into each exported SVG, and would make the
/// output indistinguishable from a project file when it is reopened.
const DOCUMENT_METADATA: &[&str] = &["desc", "metadata", "title"];

/// Whether a root-level child draws, and so must not survive isolation.
fn draws(node: &Node<'_, '_>) -> bool {
    node.is_element()
        && node.tag_name().namespace() == Some(SVG_NAMESPACE)
        && DRAWN_ELEMENTS.contains(&node.tag_name().name())
}

/// Whether a root-level child describes the project rather than the artwork.
fn describes_the_project(node: &Node<'_, '_>) -> bool {
    node.is_element()
        && node.tag_name().namespace() == Some(SVG_NAMESPACE)
        && DOCUMENT_METADATA.contains(&node.tag_name().name())
}

/// Build a standalone SVG document showing one variant at a given size.
///
/// # Errors
///
/// Returns [`crate::Error::UnknownVariant`] if the specification names a
/// variant the project does not declare, and
/// [`crate::Error::UnknownElement`] if that variant references an element the
/// document does not contain.
pub fn isolate(project: &Project, document: &Document<'_>, spec: &RenderSpec) -> Result<String> {
    build(project, document, spec, false)
}

/// The same document, minus the elements only *other* variants draw.
///
/// Not a render: the output is never rasterised, and dropping a definition
/// that mattered would show up as a preview that did not refresh rather than
/// as a broken asset. It exists to answer one question — has the variant on
/// screen changed? — and [`isolate`] cannot answer it, because it carries
/// every definition the document has, the other variants' symbols among them.
/// Compared against itself either side of an edit, that made every edit look
/// like a change to whatever happened to be on screen.
///
/// # Errors
///
/// As [`isolate`].
pub fn fingerprint(
    project: &Project,
    document: &Document<'_>,
    spec: &RenderSpec,
) -> Result<String> {
    build(project, document, spec, true)
}

/// Build the isolated document, optionally leaving out the other variants.
fn build(
    project: &Project,
    document: &Document<'_>,
    spec: &RenderSpec,
    without_other_variants: bool,
) -> Result<String> {
    let variant = project.metadata().resolve_variant(&spec.variant)?;

    // The elements no part of this variant is drawn from. Only consulted for a
    // fingerprint: a render keeps everything, because a variant is allowed to
    // `<use>` another one and nothing here resolves references.
    let others: Vec<&str> = if without_other_variants {
        project
            .metadata()
            .variants
            .iter()
            .filter(|candidate| candidate.element != variant.element)
            .map(|candidate| candidate.element.as_str())
            .collect()
    } else {
        Vec::new()
    };

    let target = document
        .descendants()
        .find(|node| node.attribute("id") == Some(variant.element.as_str()))
        .ok_or_else(|| crate::Error::UnknownElement {
            variant: variant.name.clone(),
            element: variant.element.clone(),
        })?;

    let root = document.root_element();

    // The variant's own `viewBox` when it has one — a `<symbol>` always
    // should — and the document's otherwise, which is what makes a variant
    // referencing a plain `<g>` work too.
    let view_box = target
        .attribute("viewBox")
        .or_else(|| root.attribute("viewBox"))
        .unwrap_or("0 0 100 100");

    let mut out = String::with_capacity(project.source().len());
    out.push_str(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink""#,
    );

    // Namespaces the original declared, minus the two just written, so that a
    // document using a prefix inside its definitions still parses.
    for namespace in root.namespaces() {
        match namespace.name() {
            None | Some("xlink") => {}
            Some(prefix) => {
                let _ = std::fmt::Write::write_fmt(
                    &mut out,
                    format_args!(r#" xmlns:{prefix}="{}""#, namespace.uri()),
                );
            }
        }
    }

    let _ = std::fmt::Write::write_fmt(
        &mut out,
        format_args!(
            r#" width="{}" height="{}" viewBox="{view_box}">"#,
            spec.width, spec.height
        ),
    );

    // Everything the original document defines, in its original text, so a
    // gradient or filter the variant depends on is still there.
    for child in root.children() {
        if draws(&child) || describes_the_project(&child) {
            continue;
        }
        if child.attribute("id").is_some_and(|id| others.contains(&id)) {
            continue;
        }
        if let Some(range) = child.is_element().then(|| child.range()) {
            out.push_str(&project.source()[range]);
        }
    }

    let _ = std::fmt::Write::write_fmt(
        &mut out,
        format_args!(r##"<use href="#{}"/></svg>"##, variant.element),
    );

    Ok(out)
}

#[cfg(test)]
mod tests {
    use crate::fixtures::project as fixture;

    use super::*;

    fn isolated(variant: &str, width: u32, height: u32) -> String {
        let project = fixture();
        let source = project.source().to_owned();
        let document = Document::parse(&source).unwrap();
        let mut spec = RenderSpec::square("probe", variant, width);
        spec.height = height;
        isolate(&project, &document, &spec).unwrap()
    }

    #[test]
    fn a_fingerprint_leaves_out_what_only_the_other_variants_draw() {
        // The whole point of it. An isolated document carries every definition
        // the project has, so comparing two of those either side of an edit
        // said "yes, it changed" whichever variant had actually moved — and
        // the preview re-rendered on every keystroke the agent made anywhere.
        let project = fixture();
        let source = project.source().to_owned();
        let document = Document::parse(&source).unwrap();
        let spec = RenderSpec::square("probe", "icon", 64);

        let rendered = isolate(&project, &document, &spec).unwrap();
        let compared = fingerprint(&project, &document, &spec).unwrap();

        assert!(
            rendered.contains("mark-wide"),
            "a render keeps everything, because a variant may `<use>` another"
        );
        assert!(
            !compared.contains("mark-wide"),
            "the wordmark is still in the fingerprint:\n{compared}"
        );
        assert!(compared.contains(r##"<use href="#icon"/>"##), "{compared}");
    }

    #[test]
    fn an_isolated_variant_takes_the_canvas_from_the_spec_and_the_view_box_from_the_symbol() {
        let svg = isolated("wordmark", 128, 32);
        assert!(svg.contains(r#"width="128" height="32""#), "{svg}");
        // The wordmark symbol's own viewBox, not the document's 0 0 64 64.
        assert!(svg.contains(r#"viewBox="0 0 256 64""#), "{svg}");
    }

    #[test]
    fn isolation_drops_the_documents_own_drawn_content() {
        // The root `<use href="#icon">` exists so the project file is viewable
        // in a browser. Rendering the wordmark must not also draw the icon.
        let svg = isolated("wordmark", 128, 32);
        assert_eq!(svg.matches("<use").count(), 1, "{svg}");
        assert!(
            svg.ends_with(r##"<use href="#mark-wide"/></svg>"##),
            "{svg}"
        );
    }

    #[test]
    fn isolation_keeps_every_definition_the_variant_might_depend_on() {
        let svg = isolated("icon", 64, 64);
        assert!(svg.contains(r#"<symbol id="icon""#), "{svg}");
        assert!(svg.contains(r#"<symbol id="mark-wide""#), "{svg}");
    }

    #[test]
    fn isolation_drops_the_projects_metadata() {
        // An exported asset is not a project. Carrying `<metadata>` through
        // would stamp the prompt and every render specification into each
        // exported SVG, and make the output reopen as a project.
        let svg = isolated("icon", 64, 64);
        assert!(!svg.contains("shaipe:project"), "{svg}");
        assert!(!svg.contains("<metadata"), "{svg}");
    }

    #[test]
    fn isolating_a_variant_whose_element_is_absent_says_which_element_is_missing() {
        let mut project = fixture();
        project
            .metadata_mut()
            .variants
            .push(crate::project::Variant::with_element("ghost", "nowhere"));
        let source = project.source().to_owned();
        let document = Document::parse(&source).unwrap();
        let spec = RenderSpec::square("probe", "ghost", 16);

        let error = isolate(&project, &document, &spec).unwrap_err();
        assert!(error.to_string().contains("nowhere"), "{error}");
    }
}
