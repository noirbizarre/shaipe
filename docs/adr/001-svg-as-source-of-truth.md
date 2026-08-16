# ADR-001 — The project SVG is the source of truth

## Status

Accepted

## Context

Shaipe needs somewhere to keep a prompt, a palette, a set of variants, a list
of references and a set of render specifications, alongside the artwork those
things describe.

The conventional answer is a sidecar: `logo.svg` next to `shaipe.toml`. It is
easy to implement, `serde` does all the work, and it is what most tools do.

It is also wrong for this project, for three reasons:

- **A project that is two files is a project that can be half-copied.** The
  artwork and the description of the artwork drift apart the first time
  somebody moves, renames or downloads one of them.
- **An agent editing the SVG cannot see the sidecar.** The whole premise is
  that a vision-capable model reads and writes the document. Anything it needs
  to know that lives elsewhere is something it will contradict.
- **The prompt describes the artwork.** Keeping them in separate files makes
  them separately editable, which is exactly the failure mode.

The constraint on the other side is that the file has to remain a *real* SVG.
If Shaipe's format is a dialect that Inkscape, a browser or a build pipeline
chokes on, the source of truth becomes a liability.

## Decision

The project is a single SVG file. Shaipe's data lives in a `<shaipe:project>`
element inside the document's `<metadata>`, in the namespace
`https://shaipe.dev/ns/2026`, carrying an explicit schema `version`.

Three rules follow, and are what keep the file an ordinary SVG:

1. **Variants are `<symbol>` elements**, each with its own `viewBox`. A variant
   therefore carries its own canvas rather than borrowing the document's, and
   variants do not collide.

2. **The document's root draws the primary variant**, with a `<use>`. A file
   whose artwork lives only in `<symbol>` renders *blank* in a browser and on
   GitHub. The root `<use>` is what makes `logo.svg` a logo rather than a
   container that happens to hold one.

3. **Metadata never affects geometry.** Not by convention — by construction:
   see the Consequences.

Unknown elements and attributes inside `<shaipe:project>` are ignored rather
than rejected, so a project written by a newer Shaipe stays readable. A change
older builds must *not* misread is a version bump instead, and the version is
checked before anything else is read.

### Alternatives rejected

- **A `shaipe.toml` sidecar.** Rejected above.
- **A JSON or TOML blob in a CDATA section.** `serde` would do the
  deserialising and schema evolution would be trivial. But the whole argument
  for XML is that an XML tool can traverse it, an editor can fold it, and a
  diff can show a one-line palette change as a one-line palette change. A blob
  is opaque to all three, and being opaque is what a sidecar already was.
- **Variants as top-level `<g>` elements.** Simpler, and it would have let the
  renderer use `usvg::Tree::node_by_id` directly. But every variant would then
  share the document's single `viewBox`, and all of them would draw at once in
  the root view.

## Consequences

**Two parsers read the same bytes, and this is not optional.** `usvg` discards
`<metadata>` and every foreign namespace while resolving a document, so Shaipe
metadata is invisible to the renderer's parser. It is read separately, with
`roxmltree`. The upside is rule 3 for free: the component that computes
geometry never sees the metadata, so the metadata cannot influence it.

**Writing metadata is a splice, not a serialisation.** Shaipe replaces exactly
the byte range the `<shaipe:project>` element occupied and leaves the rest of
the file alone. Re-serialising the document would be far less code and would
quietly rewrite hand-authored artwork, comments and indentation on every
palette edit. Every attribute the reader defaults is omitted when it holds that
default, so opening a project and saving it changes nothing — which is what
lets the assets check be stable.

**Rendering a variant cannot use `node_by_id`.** `usvg` resolves `<use>` by
inlining it and then drops the `<symbol>` definitions, so a variant's element
is not in the render tree to be found. This was measured, not assumed. See
ADR-003.

**The schema has to be versioned forever.** The cost of putting the format in
the artefact is that the artefact outlives the code. That is the trade being
made deliberately.
