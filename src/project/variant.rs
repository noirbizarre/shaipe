//! Named views onto parts of the project document.
//!
//! A variant is a name and an element id, nothing more. The geometry lives in
//! the document — each variant is a `<symbol>` carrying its own `viewBox` — so
//! that the metadata can never disagree with what actually renders. Nothing
//! here knows that `icon`, `wordmark` and `watermark` are common names for
//! variants, because a project is free to invent its own.

/// A named part of the project document that can be rendered on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    /// The stable name render specifications refer to.
    pub name: String,
    /// The id of the element in the document that draws it.
    pub element: String,
}

impl Variant {
    /// A variant named after the element it references.
    ///
    /// The overwhelmingly common case, and keeping it a constructor stops
    /// every call site repeating the string.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            element: name.clone(),
            name,
        }
    }

    /// A variant whose name and element id differ.
    #[must_use]
    pub fn with_element(name: impl Into<String>, element: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            element: element.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn a_variant_defaults_to_referencing_the_element_it_is_named_after() {
        let variant = Variant::new("icon");
        assert_eq!(variant.name, "icon");
        assert_eq!(variant.element, "icon");
    }
}
