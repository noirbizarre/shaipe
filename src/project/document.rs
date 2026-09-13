//! The raw-XML layer: reading a project file, and writing metadata back into it.
//!
//! Shaipe never regenerates the project file. It holds the original bytes and,
//! when metadata changes, replaces exactly the byte range the
//! `<shaipe:project>` element occupied. Everything else — the artwork, the
//! comments, the author's indentation, the attribute order — survives
//! untouched.
//!
//! Serialising the whole document instead would be far less code and would
//! quietly rewrite hand-authored artwork on every palette edit. For a file
//! that is meant to be the source of truth, and to live in Git next to the
//! things it generates, that is the wrong trade.

use std::fmt::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};

use roxmltree::{Document, Node};

use crate::error::{Error, Result};
use crate::project::metadata::{Metadata, NAMESPACE, SCHEMA_VERSION};
use crate::project::palette::Rgba;
use crate::project::spec::{Background, Format};

/// The SVG namespace a project document's root must be in.
pub const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

/// Parse an SVG document, checking it really is one.
///
/// # Errors
///
/// Returns [`Error::MalformedXml`] when the bytes are not XML, and
/// [`Error::NotSvg`] when the root element is not an SVG `<svg>`.
pub fn parse<'a>(source: &'a str, path: &Path) -> Result<Document<'a>> {
    let document = Document::parse(source).map_err(|source| Error::MalformedXml {
        path: path.to_path_buf(),
        source,
    })?;

    if !document.root_element().has_tag_name((SVG_NAMESPACE, "svg")) {
        return Err(Error::NotSvg {
            path: path.to_path_buf(),
        });
    }

    Ok(document)
}

/// Produce the bytes of `source` with its metadata element replaced.
///
/// # Errors
///
/// Returns [`Error::MalformedXml`] or [`Error::NoMetadata`] if `source` is not
/// a project document, and [`Error::WriteMetadata`] if serialisation fails.
pub fn replace_metadata(source: &str, metadata: &Metadata, path: &Path) -> Result<String> {
    let document = parse(source, path)?;
    let element = document
        .descendants()
        .find(|node| node.has_tag_name((NAMESPACE, "project")))
        .ok_or_else(|| Error::NoMetadata {
            path: path.to_path_buf(),
        })?;
    let range = element.range();

    // Redeclared only if no ancestor already binds the namespace. Emitting it
    // unconditionally is harmless XML but rewrites documents that declare it
    // on `<svg>` — the idiomatic place — and so breaks the guarantee that
    // opening a project and saving it changes nothing.
    let declare_namespace = element
        .parent()
        .and_then(|parent| parent.lookup_prefix(NAMESPACE))
        .is_none();

    // The metadata element's own indentation, so the replacement lines up with
    // whatever the rest of the file does rather than imposing a house style.
    let indent = source[..range.start]
        .rsplit('\n')
        .next()
        .filter(|line| line.chars().all(char::is_whitespace))
        .unwrap_or_default()
        .to_owned();

    let serialised = serialise(metadata, &indent, declare_namespace);

    let mut output = String::with_capacity(source.len() + serialised.len());
    output.push_str(&source[..range.start]);
    output.push_str(&serialised);
    output.push_str(&source[range.end..]);
    Ok(output)
}

/// Produce the bytes of `source` with the element carrying `id` replaced.
///
/// Splices by byte range, the same technique [`replace_metadata`] uses for
/// `<shaipe:project>`: only the target element's own bytes change, so a
/// sibling's hand-authored formatting, comments and unrelated definitions
/// survive untouched.
///
/// `variant` names the caller's variant for [`Error::UnknownElement`] only;
/// nothing here reads the metadata, so it cannot resolve a variant to an
/// element id itself — that is the caller's job.
///
/// # Errors
///
/// Returns [`Error::MalformedXml`] or [`Error::NotSvg`] if `source` does not
/// parse, and [`Error::UnknownElement`] if nothing in the document carries
/// `id`.
pub fn replace_element(
    source: &str,
    variant: &str,
    id: &str,
    replacement: &str,
    path: &Path,
) -> Result<String> {
    let document = parse(source, path)?;
    let element = document
        .descendants()
        .find(|node| node.attribute("id") == Some(id))
        .ok_or_else(|| Error::UnknownElement {
            variant: variant.to_owned(),
            element: id.to_owned(),
        })?;
    let range = element.range();

    let mut output = String::with_capacity(source.len() + replacement.len());
    output.push_str(&source[..range.start]);
    output.push_str(replacement);
    output.push_str(&source[range.end..]);
    Ok(output)
}

/// Restyle every element bound to a palette colour, wherever it is.
///
/// Binding is a `shaipe:fill`/`shaipe:stroke` attribute naming a colour by its
/// palette name, read alongside the element's own `fill`/`stroke` rather than
/// instead of it — so the document stays an ordinary SVG whether or not
/// anything ever calls this. Only the plain attribute's value changes; the
/// binding attribute is left exactly as it was, and so is everything else in
/// the document, splice by splice, the same technique [`replace_element`]
/// uses for a single element.
///
/// A colour with nothing bound to it is the common case, and costs one copy
/// of `source` with nothing changed.
///
/// # Errors
///
/// Returns [`Error::MalformedXml`] or [`Error::NotSvg`] if `source` does not
/// parse — unreachable for a project that already opened, since this is
/// always called on a document Shaipe itself just read.
pub fn apply_binding(source: &str, name: &str, value: Rgba, path: &Path) -> Result<String> {
    let document = parse(source, path)?;
    let value = value.to_string();

    // Collected before anything is written, and keyed by byte range rather
    // than by node, so elements bound more than once in the document (an icon
    // and its inverse, say) are all found in one pass over the tree.
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();

    for node in document.descendants().filter(Node::is_element) {
        for attribute in ["fill", "stroke"] {
            let Some(binding) = node.attribute((NAMESPACE, attribute)) else {
                continue;
            };
            if binding != name {
                continue;
            }

            // `Node::attribute_node` matches by local name alone once no
            // namespace is asked for, which would find `shaipe:fill` itself
            // when there is no plain `fill` to find — the two share a local
            // name and differ only by prefix. The unnamespaced attribute has
            // to be found explicitly instead.
            let plain = node
                .attributes()
                .find(|candidate| candidate.name() == attribute && candidate.namespace().is_none());

            match plain {
                // The common case: the element already carries a literal
                // colour, and only its value needs to change.
                Some(existing) => edits.push((existing.range_value(), value.clone())),
                // Bound but with nothing to fall back on yet — an attribute is
                // inserted right after the binding, so the element still
                // renders correctly wherever `shaipe:fill` itself is not
                // understood.
                None => {
                    let binding_attribute = node
                        .attribute_node((NAMESPACE, attribute))
                        .expect("just matched its value");
                    let at = binding_attribute.range().end;
                    edits.push((at..at, format!(r#" {attribute}="{value}""#)));
                }
            }
        }
    }

    if edits.is_empty() {
        return Ok(source.to_owned());
    }

    edits.sort_by_key(|(range, _)| range.start);

    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    for (range, replacement) in edits {
        output.push_str(&source[cursor..range.start]);
        output.push_str(&replacement);
        cursor = range.end;
    }
    output.push_str(&source[cursor..]);

    Ok(output)
}

/// Escape the five characters XML will not accept as text or in an attribute.
fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Render `<shaipe:project>` as XML, indented to sit at `indent`.
///
/// Written by hand rather than derived. The output is read by humans and by
/// `git diff`, the vocabulary is a dozen elements, and a serialiser that
/// guessed at element-versus-attribute placement would need more annotation
/// than this is code.
///
/// Every attribute the reader supplies a default for is omitted when it holds
/// that default. Writing them all out would be simpler and would turn every
/// hand-authored project into a diff the first time Shaipe touched it.
fn serialise(metadata: &Metadata, indent: &str, declare_namespace: bool) -> String {
    let mut out = String::new();
    let step = "  ";
    let l1 = format!("{indent}{step}");
    let l2 = format!("{l1}{step}");

    out.push_str("<shaipe:project");
    if declare_namespace {
        let _ = write!(out, r#" xmlns:shaipe="{NAMESPACE}""#);
    }
    let _ = write!(out, r#" version="{SCHEMA_VERSION}""#);
    if let Some(primary) = &metadata.primary {
        let _ = write!(out, r#" primary="{}""#, escape(primary));
    }
    out.push('>');

    if let Some(prompt) = &metadata.prompt {
        let _ = write!(
            out,
            "\n{l1}<shaipe:prompt>{}</shaipe:prompt>",
            escape(prompt)
        );
    }

    if !metadata.generation.is_empty() {
        let _ = write!(out, "\n{l1}<shaipe:generation");
        for (name, value) in [
            ("agent", &metadata.generation.agent),
            ("model", &metadata.generation.model),
            ("at", &metadata.generation.at),
        ] {
            if let Some(value) = value {
                let _ = write!(out, r#" {name}="{}""#, escape(value));
            }
        }
        out.push_str("/>");
    }

    if !metadata.palette.is_empty() {
        let _ = write!(out, "\n{l1}<shaipe:palette>");
        for colour in metadata.palette.colours() {
            let _ = write!(
                out,
                r#"{}<shaipe:color name="{}" value="{}""#,
                format_args!("\n{l2}"),
                escape(&colour.name),
                colour.value
            );
            if let Some(role) = &colour.role {
                let _ = write!(out, r#" role="{}""#, escape(&role.to_string()));
            }
            out.push_str("/>");
        }
        let _ = write!(out, "\n{l1}</shaipe:palette>");
    }

    if !metadata.fonts.is_empty() {
        let _ = write!(out, "\n{l1}<shaipe:fonts>");
        for font in &metadata.fonts {
            let _ = write!(
                out,
                r#"{}<shaipe:font family="{}" src="{}"/>"#,
                format_args!("\n{l2}"),
                escape(&font.family),
                escape(&font.src.display().to_string())
            );
        }
        let _ = write!(out, "\n{l1}</shaipe:fonts>");
    }

    if !metadata.variants.is_empty() {
        let _ = write!(out, "\n{l1}<shaipe:variants>");
        for variant in &metadata.variants {
            let _ = write!(
                out,
                r#"{}<shaipe:variant name="{}""#,
                format_args!("\n{l2}"),
                escape(&variant.name)
            );
            // Omitted when it is the default, so the common case stays quiet.
            if variant.element != variant.name {
                let _ = write!(out, r#" ref="{}""#, escape(&variant.element));
            }
            out.push_str("/>");
        }
        let _ = write!(out, "\n{l1}</shaipe:variants>");
    }

    if !metadata.references.is_empty() {
        let _ = write!(out, "\n{l1}<shaipe:references>");
        for reference in &metadata.references {
            let _ = write!(
                out,
                r#"{}<shaipe:reference src="{}" kind="{}""#,
                format_args!("\n{l2}"),
                escape(&reference.src.display().to_string()),
                escape(&reference.kind.to_string())
            );
            match &reference.note {
                Some(note) => {
                    let _ = write!(out, ">{}</shaipe:reference>", escape(note));
                }
                None => out.push_str("/>"),
            }
        }
        let _ = write!(out, "\n{l1}</shaipe:references>");
    }

    if !metadata.renders.is_empty() {
        let _ = write!(out, "\n{l1}<shaipe:renders");
        if let Some(output) = &metadata.render_output {
            let _ = write!(
                out,
                r#" output="{}""#,
                escape(&output.display().to_string())
            );
        }
        out.push('>');
        for spec in &metadata.renders {
            let _ = write!(
                out,
                r#"{}<shaipe:render name="{}" variant="{}" width="{}""#,
                format_args!("\n{l2}"),
                escape(&spec.name),
                escape(&spec.variant),
                spec.width,
            );
            // A lone `width` means a square; see `metadata::read_renders`.
            if spec.height != spec.width {
                let _ = write!(out, r#" height="{}""#, spec.height);
            }
            if spec.format != Format::default() {
                let _ = write!(out, r#" format="{}""#, spec.format);
            }
            if spec.background != Background::default() {
                let _ = write!(out, r#" background="{}""#, spec.background);
            }
            out.push_str("/>");
        }
        let _ = write!(out, "\n{l1}</shaipe:renders>");
    }

    let _ = write!(out, "\n{indent}</shaipe:project>");
    out
}

/// Read a file, naming it if that fails.
///
/// # Errors
///
/// Returns [`Error::NotAFile`] if `path` is a directory — a directory is not
/// a project file, and the OS's own message for that ("Is a directory") does
/// not say so — and [`Error::Io`] for every other failure to read it.
pub fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::IsADirectory {
            Error::NotAFile {
                path: path.to_path_buf(),
            }
        } else {
            Error::io(path, source)
        }
    })
}

/// Write a file, naming it if that fails.
///
/// # Errors
///
/// Returns [`Error::Io`] carrying the path.
pub fn write(path: &Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents).map_err(|source| Error::io(path, source))
}

/// The directory paths in a project's metadata are resolved against.
#[must_use]
pub fn base_directory(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::project::palette::{Colour, Rgba};
    use crate::project::variant::Variant;

    const MINIMAL: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">
  <metadata>
    <shaipe:project xmlns:shaipe="https://shaipe.dev/ns/2026" version="1" primary="icon">
      <shaipe:variants>
        <shaipe:variant name="icon"/>
      </shaipe:variants>
    </shaipe:project>
  </metadata>
  <!-- a comment the author cares about -->
  <symbol id="icon" viewBox="0 0 16 16"><rect width="16" height="16"/></symbol>
  <use href="#icon"/>
</svg>
"##;

    fn parse_metadata(source: &str) -> Metadata {
        let path = Path::new("test.svg");
        let document = parse(source, path).unwrap();
        Metadata::from_document(&document, path).unwrap()
    }

    #[test]
    fn a_document_whose_root_is_not_svg_is_rejected() {
        let error = parse("<html><body/></html>", Path::new("x.html")).unwrap_err();
        assert!(matches!(error, Error::NotSvg { .. }));
    }

    #[test]
    fn reading_a_directory_says_so_rather_than_the_oss_raw_message() {
        let directory = tempfile::tempdir().unwrap();
        let error = read(directory.path()).unwrap_err();
        assert!(matches!(error, Error::NotAFile { .. }), "{error:?}");
    }

    #[test]
    fn replacing_metadata_leaves_the_rest_of_the_document_byte_identical() {
        // The whole reason for splicing by byte range rather than
        // re-serialising: hand-authored artwork and comments must survive.
        let mut metadata = parse_metadata(MINIMAL);
        metadata.palette.push(Colour {
            name: "accent".to_owned(),
            value: Rgba::new(0xf0, 0x50, 0x32, 0xff),
            role: None,
        });

        let updated = replace_metadata(MINIMAL, &metadata, Path::new("test.svg")).unwrap();

        assert!(updated.contains("<!-- a comment the author cares about -->"));
        assert!(updated.contains(
            r#"<symbol id="icon" viewBox="0 0 16 16"><rect width="16" height="16"/></symbol>"#
        ));
        assert!(updated.contains(r##"<shaipe:color name="accent" value="#f05032"/>"##));
    }

    #[test]
    fn metadata_survives_a_write_then_read_round_trip() {
        let mut original = parse_metadata(MINIMAL);
        original.prompt = Some("A mark with <angle> & \"quote\"".to_owned());
        original
            .variants
            .push(Variant::with_element("mark", "icon"));
        original.palette.push(Colour {
            name: "accent".to_owned(),
            value: Rgba::new(0xf0, 0x50, 0x32, 0x80),
            role: Some(crate::project::palette::Role::Accent),
        });

        let updated = replace_metadata(MINIMAL, &original, Path::new("test.svg")).unwrap();
        assert_eq!(parse_metadata(&updated), original);
    }

    #[test]
    fn rewriting_unchanged_metadata_is_idempotent() {
        // Otherwise `shaipe` would dirty the working tree just by opening a
        // project, and the assets CI check would never be stable.
        let metadata = parse_metadata(MINIMAL);
        let once = replace_metadata(MINIMAL, &metadata, Path::new("test.svg")).unwrap();
        let twice = replace_metadata(&once, &metadata, Path::new("test.svg")).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn replacing_an_element_by_id_leaves_the_rest_of_the_document_byte_identical() {
        // The same guarantee `replace_metadata` gives the `<shaipe:project>`
        // block, extended to an arbitrary element: only the target's own
        // bytes change.
        let updated = replace_element(
            MINIMAL,
            "icon",
            "icon",
            r#"<symbol id="icon" viewBox="0 0 16 16"><circle r="8" cx="8" cy="8"/></symbol>"#,
            Path::new("test.svg"),
        )
        .unwrap();

        assert!(updated.contains("<!-- a comment the author cares about -->"));
        assert!(updated.contains(r#"<circle r="8" cx="8" cy="8"/>"#));
        assert!(!updated.contains(r#"<rect width="16" height="16"/>"#));
        // Everything outside the replaced element, including the metadata,
        // is untouched.
        assert!(updated.contains(r#"<shaipe:project xmlns:shaipe="https://shaipe.dev/ns/2026" version="1" primary="icon">"#));
    }

    #[test]
    fn replacing_an_element_that_does_not_exist_names_the_variant_and_the_id() {
        let error = replace_element(MINIMAL, "ghost", "nowhere", "<g/>", Path::new("test.svg"))
            .unwrap_err();

        assert!(matches!(error, Error::UnknownElement { .. }));
        let rendered = error.to_string();
        assert!(rendered.contains("ghost"), "{rendered}");
        assert!(rendered.contains("nowhere"), "{rendered}");
    }

    /// Two elements bound to `accent` — one with a literal `fill` already,
    /// one without — and a third bound to a different colour entirely, so a
    /// test can tell "restyled" apart from "untouched".
    const BOUND: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:shaipe="https://shaipe.dev/ns/2026" viewBox="0 0 16 16">
  <metadata>
    <shaipe:project version="1" primary="icon">
      <shaipe:palette>
        <shaipe:color name="accent" value="#f05032"/>
        <shaipe:color name="ink" value="#18181b"/>
      </shaipe:palette>
      <shaipe:variants>
        <shaipe:variant name="icon"/>
      </shaipe:variants>
    </shaipe:project>
  </metadata>
  <!-- a comment the author cares about -->
  <symbol id="icon" viewBox="0 0 16 16">
    <rect width="16" height="16" fill="#f05032" shaipe:fill="accent"/>
    <circle r="4" cx="8" cy="8" shaipe:fill="accent"/>
    <path d="M0 0 L16 16" stroke="#18181b" shaipe:stroke="ink"/>
  </symbol>
  <use href="#icon"/>
</svg>
"##;

    #[test]
    fn binding_rewrites_an_existing_literal_attribute_in_place() {
        let updated = apply_binding(
            BOUND,
            "accent",
            Rgba::new(0, 0, 0, 0xff),
            Path::new("t.svg"),
        )
        .unwrap();

        assert!(
            updated.contains(
                r##"<rect width="16" height="16" fill="#000000" shaipe:fill="accent"/>"##
            ),
            "{updated}"
        );
        // The palette's own record of the colour is untouched — restyling
        // artwork and updating the palette are two separate steps, and this
        // one only does the first.
        assert!(
            updated.contains(r##"<shaipe:color name="accent" value="#f05032"/>"##),
            "{updated}"
        );
    }

    #[test]
    fn binding_inserts_a_literal_attribute_when_the_element_has_none() {
        let updated = apply_binding(
            BOUND,
            "accent",
            Rgba::new(0, 0, 0, 0xff),
            Path::new("t.svg"),
        )
        .unwrap();

        assert!(
            updated
                .contains(r##"<circle r="4" cx="8" cy="8" shaipe:fill="accent" fill="#000000"/>"##),
            "{updated}"
        );
    }

    #[test]
    fn binding_touches_only_elements_bound_to_the_edited_name() {
        let updated = apply_binding(
            BOUND,
            "accent",
            Rgba::new(0, 0, 0, 0xff),
            Path::new("t.svg"),
        )
        .unwrap();

        // `ink` was not the colour being edited, so its own binding is left
        // exactly as it read.
        assert!(
            updated.contains(r##"<path d="M0 0 L16 16" stroke="#18181b" shaipe:stroke="ink"/>"##),
            "{updated}"
        );
    }

    #[test]
    fn binding_leaves_the_rest_of_the_document_byte_identical() {
        let updated = apply_binding(
            BOUND,
            "accent",
            Rgba::new(0, 0, 0, 0xff),
            Path::new("t.svg"),
        )
        .unwrap();

        assert!(updated.contains("<!-- a comment the author cares about -->"));
        assert!(updated.contains(r##"<shaipe:color name="accent" value="#f05032"/>"##));
    }

    #[test]
    fn binding_a_colour_nothing_references_leaves_the_document_unchanged() {
        let updated = apply_binding(
            BOUND,
            "unused",
            Rgba::new(0, 0, 0, 0xff),
            Path::new("t.svg"),
        )
        .unwrap();
        assert_eq!(updated, BOUND);
    }
}
