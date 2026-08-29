//! What a tool produces.
//!
//! Two channels, because an image is not a JSON value. Base64 inside a JSON
//! string is a string as far as every transport is concerned, and a model
//! cannot see a string. MCP carries images as their own content blocks, so the
//! bytes travel *beside* the JSON rather than inside it, and the one thing
//! Shaipe exists to do — let an agent look at the artwork — actually works.

use serde_json::Value;

/// One rendered image, on its way to whoever called the tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolImage {
    /// What it shows, in a few words: `icon at 16x16`.
    ///
    /// Emitted as text immediately before the image, because a bare image in a
    /// transcript has nothing to say which of five sizes it is.
    pub label: String,
    /// `image/png` for a render; whatever a reference file's extension
    /// implies otherwise. Named rather than assumed, so every transport
    /// already carries the right answer instead of guessing at one.
    pub mime_type: &'static str,
    /// The encoded image.
    ///
    /// Raw bytes, not base64: the workspace wants to *show* this image, and
    /// making it decode base64 to do so would be encoding for a transport that
    /// is not involved. Whoever needs base64 encodes it themselves.
    pub bytes: Vec<u8>,
}

impl ToolImage {
    /// An image, labelled and in a known format.
    #[must_use]
    pub fn new(label: impl Into<String>, mime_type: &'static str, bytes: Vec<u8>) -> Self {
        Self {
            label: label.into(),
            mime_type,
            bytes,
        }
    }

    /// A PNG, labelled.
    #[must_use]
    pub fn png(label: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self::new(label, "image/png", bytes)
    }
}

/// The result of calling a tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    /// The structured answer. Always present, even when the point is the
    /// images: it is what says which variant was drawn and at what size.
    pub value: Value,
    /// Anything the caller should look at, in the order it should be seen.
    pub images: Vec<ToolImage>,
}

impl ToolOutput {
    /// An answer with nothing to look at.
    #[must_use]
    pub const fn json(value: Value) -> Self {
        Self {
            value,
            images: Vec::new(),
        }
    }

    /// Add an image.
    #[must_use]
    pub fn with_image(mut self, image: ToolImage) -> Self {
        self.images.push(image);
        self
    }

    /// Add several.
    #[must_use]
    pub fn with_images(mut self, images: impl IntoIterator<Item = ToolImage>) -> Self {
        self.images.extend(images);
        self
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::*;

    #[test]
    fn an_image_carries_its_bytes_rather_than_a_base64_string() {
        // The whole reason this type exists. If this ever holds a `String`,
        // the workspace has to decode base64 to draw its own render.
        let image = ToolImage::png("icon at 16x16", vec![0x89, b'P', b'N', b'G']);
        assert_eq!(image.bytes, vec![0x89, b'P', b'N', b'G']);
        assert_eq!(image.mime_type, "image/png");
    }

    #[test]
    fn images_keep_the_order_they_were_added_in() {
        // `render_grid` returns largest first on purpose; a set that reorders
        // itself would make the model's reading of it arbitrary.
        let output = ToolOutput::json(json!({}))
            .with_image(ToolImage::png("first", vec![1]))
            .with_image(ToolImage::png("second", vec![2]));

        let labels: Vec<_> = output.images.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["first", "second"]);
    }
}
