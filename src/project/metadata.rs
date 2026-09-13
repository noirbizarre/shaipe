//! The Shaipe metadata schema, and how it is read from and written to an SVG.
//!
//! # Why this is not `usvg`'s job
//!
//! `usvg` resolves an SVG into a render tree, and in doing so discards
//! `<metadata>` and every element in a foreign namespace. Shaipe metadata is
//! therefore invisible to the renderer's parser by construction, and has to be
//! read by a second pass over the same bytes. That is a feature as much as a
//! constraint: it guarantees metadata cannot influence geometry, because the
//! thing that computes geometry never sees it.
//!
//! # Why XML rather than a JSON blob
//!
//! The project file has to remain a valid, useful SVG that other tools can
//! open. Real elements in a declared namespace are what an XML tool can
//! traverse, an editor can fold, and `git diff` can show a one-line change in.
//! A CDATA blob would be easier to deserialise and worse at all three.
//!
//! # Evolution
//!
//! `<shaipe:project version="N">` is checked before anything else is read. An
//! unknown attribute or child element is ignored rather than rejected, so a
//! project written by a newer Shaipe stays readable; a change that older
//! builds must *not* misread is a version bump instead.

use std::path::{Path, PathBuf};

use roxmltree::{Document, Node};

use crate::error::{Error, Result};
use crate::project::font::Font;
use crate::project::palette::{Colour, Palette, Role};
use crate::project::reference::{Reference, ReferenceKind};
use crate::project::spec::{Background, Format, RenderSpec};
use crate::project::variant::Variant;

/// The namespace Shaipe metadata lives in.
///
/// Versioned by year rather than by schema version: the namespace identifies
/// *whose* vocabulary this is, and `version` on the root element identifies
/// which revision of it. Changing the namespace would orphan every existing
/// project, so it does not change.
pub const NAMESPACE: &str = "https://shaipe.dev/ns/2026";

/// The schema revision this build reads and writes.
pub const SCHEMA_VERSION: u32 = 1;

/// How a project came to look the way it does.
///
/// Recorded by whatever agent edited the project, never invented by Shaipe.
/// It is absent until something real has written it, which is why every field
/// is optional and why there is no `Default` that fills in a timestamp.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Generation {
    /// The agent that produced the current state, for example `opencode`.
    pub agent: Option<String>,
    /// The model it used.
    pub model: Option<String>,
    /// When, as an RFC 3339 timestamp written by that agent.
    pub at: Option<String>,
}

impl Generation {
    /// Whether anything was recorded at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.agent.is_none() && self.model.is_none() && self.at.is_none()
    }
}

/// Everything Shaipe knows about a project that is not its geometry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    /// The variant the document's root draws, keeping the file viewable.
    pub primary: Option<String>,
    /// What the project is meant to be, in the author's words.
    pub prompt: Option<String>,
    /// How the current state was produced.
    pub generation: Generation,
    /// The project's colours.
    pub palette: Palette,
    /// Fonts the document's text depends on.
    pub fonts: Vec<Font>,
    /// Named parts of the document that can be rendered on their own.
    pub variants: Vec<Variant>,
    /// Images and documents the project was informed by.
    pub references: Vec<Reference>,
    /// Assets the project declares.
    pub renders: Vec<RenderSpec>,
    /// Where a bare `shaipe render` writes those assets, if the project says.
    ///
    /// Relative to the project file, the same as every other path here — see
    /// [`crate::project::Project::resolve`]. `--output` on the command line
    /// still overrides it; this is only the fallback for when nothing else
    /// says where assets belong.
    pub render_output: Option<PathBuf>,
}

impl Metadata {
    /// Look a variant up by name.
    #[must_use]
    pub fn variant(&self, name: &str) -> Option<&Variant> {
        self.variants.iter().find(|variant| variant.name == name)
    }

    /// Look a render specification up by name.
    #[must_use]
    pub fn render_spec(&self, name: &str) -> Option<&RenderSpec> {
        self.renders.iter().find(|spec| spec.name == name)
    }

    /// The names of every declared variant, for error messages.
    #[must_use]
    pub fn variant_names(&self) -> Vec<String> {
        self.variants
            .iter()
            .map(|variant| variant.name.clone())
            .collect()
    }

    /// The names of every declared render specification, for error messages.
    #[must_use]
    pub fn spec_names(&self) -> Vec<String> {
        self.renders.iter().map(|spec| spec.name.clone()).collect()
    }

    /// Resolve a variant name, or say what does exist.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownVariant`], listing the declared variants.
    pub fn resolve_variant(&self, name: &str) -> Result<&Variant> {
        self.variant(name).ok_or_else(|| Error::UnknownVariant {
            variant: name.to_owned(),
            known: self.variant_names(),
        })
    }

    /// Read the metadata out of an already-parsed SVG document.
    ///
    /// `path` is carried only so that failures can name the file.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NoMetadata`] when the document declares none,
    /// [`Error::UnsupportedSchema`] when it declares a version this build does
    /// not implement, and [`Error::InvalidMetadata`] when an element is
    /// present but unreadable.
    pub fn from_document(document: &Document<'_>, path: &Path) -> Result<Self> {
        let root = document
            .descendants()
            .find(|node| node.has_tag_name((NAMESPACE, "project")))
            .ok_or_else(|| Error::NoMetadata {
                path: path.to_path_buf(),
            })?;

        // Checked before any child is read: a document from the future may
        // reuse an element name for something incompatible, and reading it
        // with this build's assumptions would corrupt it on the next save.
        let declared = root.attribute("version").unwrap_or_default();
        let version: u32 = declared.parse().map_err(|_| Error::UnsupportedSchema {
            path: path.to_path_buf(),
            found: declared.to_owned(),
            supported: SCHEMA_VERSION,
        })?;
        if version != SCHEMA_VERSION {
            return Err(Error::UnsupportedSchema {
                path: path.to_path_buf(),
                found: declared.to_owned(),
                supported: SCHEMA_VERSION,
            });
        }

        let mut metadata = Self {
            primary: root.attribute("primary").map(str::to_owned),
            ..Self::default()
        };

        for child in root.children().filter(Node::is_element) {
            match child.tag_name().name() {
                "prompt" => metadata.prompt = child.text().map(|text| text.trim().to_owned()),
                "generation" => metadata.generation = read_generation(&child),
                "palette" => metadata.palette = read_palette(&child, path)?,
                "fonts" => metadata.fonts = read_fonts(&child, path)?,
                "variants" => metadata.variants = read_variants(&child, path)?,
                "references" => metadata.references = read_references(&child, path)?,
                "renders" => {
                    metadata.renders = read_renders(&child, path)?;
                    metadata.render_output = child.attribute("output").map(PathBuf::from);
                }
                // Ignored on purpose. See the module documentation: forwards
                // compatibility is the reason the schema is versioned at all.
                _ => {}
            }
        }

        Ok(metadata)
    }
}

fn read_generation(node: &Node<'_, '_>) -> Generation {
    Generation {
        agent: node.attribute("agent").map(str::to_owned),
        model: node.attribute("model").map(str::to_owned),
        at: node.attribute("at").map(str::to_owned),
    }
}

/// Require an attribute, naming the element that should have carried it.
fn require<'a>(node: &Node<'a, '_>, name: &str, path: &Path) -> Result<&'a str> {
    node.attribute(name).ok_or_else(|| Error::InvalidMetadata {
        path: path.to_path_buf(),
        reason: format!(
            "`<shaipe:{}>` is missing the `{name}` attribute",
            node.tag_name().name()
        ),
    })
}

fn read_palette(node: &Node<'_, '_>, path: &Path) -> Result<Palette> {
    node.children()
        .filter(|child| child.has_tag_name((NAMESPACE, "color")))
        .map(|child| {
            Ok(Colour {
                name: require(&child, "name", path)?.to_owned(),
                value: require(&child, "value", path)?.parse()?,
                role: child.attribute("role").map(Role::from),
            })
        })
        .collect()
}

fn read_fonts(node: &Node<'_, '_>, path: &Path) -> Result<Vec<Font>> {
    node.children()
        .filter(|child| child.has_tag_name((NAMESPACE, "font")))
        .map(|child| {
            let family = require(&child, "family", path)?;
            let invalid = |reason: String| Error::InvalidMetadata {
                path: path.to_path_buf(),
                reason,
            };

            match (child.attribute("src"), child.attribute("href")) {
                (Some(src), None) => Ok(Font::new(family, PathBuf::from(src))),
                (None, Some(href)) => {
                    // Unpinned would just be system-font fallback with extra
                    // steps — the cache key (and the whole point of caching)
                    // is the hash the project commits to, not the URL.
                    let sha256 = child.attribute("sha256").ok_or_else(|| {
                        invalid(format!(
                            "`<shaipe:font family=\"{family}\">` has `href` but no `sha256` — \
                             a remote font must pin the checksum it expects"
                        ))
                    })?;
                    Ok(Font::remote(family, href, sha256))
                }
                (Some(_), Some(_)) => Err(invalid(format!(
                    "`<shaipe:font family=\"{family}\">` has both `src` and `href` — \
                     a font is one or the other, not both"
                ))),
                (None, None) => Err(invalid(format!(
                    "`<shaipe:font family=\"{family}\">` has neither `src` nor `href`"
                ))),
            }
        })
        .collect()
}

fn read_variants(node: &Node<'_, '_>, path: &Path) -> Result<Vec<Variant>> {
    node.children()
        .filter(|child| child.has_tag_name((NAMESPACE, "variant")))
        .map(|child| {
            let name = require(&child, "name", path)?;
            // `ref` defaults to `name`, because a variant whose element is
            // named after it is the norm and repeating it is noise.
            Ok(Variant::with_element(
                name,
                child.attribute("ref").unwrap_or(name),
            ))
        })
        .collect()
}

fn read_references(node: &Node<'_, '_>, path: &Path) -> Result<Vec<Reference>> {
    node.children()
        .filter(|child| child.has_tag_name((NAMESPACE, "reference")))
        .map(|child| {
            Ok(Reference {
                src: PathBuf::from(require(&child, "src", path)?),
                kind: child
                    .attribute("kind")
                    .map_or(ReferenceKind::Inspiration, ReferenceKind::from),
                note: child
                    .text()
                    .map(|text| text.trim().to_owned())
                    .filter(|text| !text.is_empty()),
            })
        })
        .collect()
}

fn read_renders(node: &Node<'_, '_>, path: &Path) -> Result<Vec<RenderSpec>> {
    node.children()
        .filter(|child| child.has_tag_name((NAMESPACE, "render")))
        .map(|child| {
            let name = require(&child, "name", path)?;

            let dimension = |attribute: &str, fallback: Option<&str>| -> Result<u32> {
                let raw = child.attribute(attribute).or(fallback).ok_or_else(|| {
                    Error::InvalidMetadata {
                        path: path.to_path_buf(),
                        reason: format!(
                            "`<shaipe:render name=\"{name}\">` is missing the `{attribute}` attribute"
                        ),
                    }
                })?;
                raw.parse().map_err(|_| Error::InvalidMetadata {
                    path: path.to_path_buf(),
                    reason: format!(
                        "`<shaipe:render name=\"{name}\">` has `{attribute}=\"{raw}\"`, which is not a pixel count"
                    ),
                })
            };

            let width = child.attribute("width");
            // A single `width` means a square, which is what almost every icon
            // specification wants and what every one of them would otherwise
            // have to say twice.
            let height = dimension("height", width)?;
            let width = dimension("width", child.attribute("height"))?;

            Ok(RenderSpec {
                name: name.to_owned(),
                variant: require(&child, "variant", path)?.to_owned(),
                width,
                height,
                format: child
                    .attribute("format")
                    .map_or(Ok(Format::default()), str::parse)
                    .map_err(|reason| Error::InvalidMetadata {
                        path: path.to_path_buf(),
                        reason: format!("`<shaipe:render name=\"{name}\">`: {reason}"),
                    })?,
                background: child
                    .attribute("background")
                    .map_or(Ok(Background::default()), str::parse)?,
            })
        })
        .collect()
}
