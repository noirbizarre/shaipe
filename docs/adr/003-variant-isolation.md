# ADR-003 — A variant is rendered by isolating it, not by rendering a node

## Status

Accepted

## Context

Given a project holding several variants, the renderer has to draw exactly one
of them, on a canvas of a requested size, with the right aspect ratio.

`resvg` appears to offer this directly: `usvg::Tree::node_by_id` finds a node,
`resvg::render_node` draws it. That was the intended implementation.

It does not work. `usvg` resolves `<use>` by inlining the referenced content
and then discards the `<symbol>` definitions entirely — they are not renderable
elements, so nothing survives to be found. Probed against `usvg` 0.48.1:

```text
node_by_id(icon)      = NONE
node_by_id(mark-wide) = NONE
```

The inlined group does not keep the id either, so there is nothing to match on.

Rendering a node also would not have solved the harder half of the problem.
`render_node` draws a node's own geometry; fitting that geometry into a
requested canvas, preserving its aspect ratio and centring it, would be
arithmetic in Shaipe — and floating-point arithmetic that is wrong by half a
pixel is exactly the kind of bug that is invisible until it is in a favicon.

## Decision

Isolate the variant at the XML level, before `usvg` ever sees it, and render
the result as an ordinary complete document.

The isolated document is built from the project's own bytes:

- everything the original defines is kept verbatim — `<defs>`, `<symbol>`,
  gradients, filters — so anything the variant references is still there;
- the root-level elements that *draw* are dropped, so the `<use>` that makes
  the project file viewable does not also appear in a render of some other
  variant;
- `<metadata>`, `<title>` and `<desc>` are dropped, because an exported asset
  is not a project and should not carry the prompt;
- a single `<use>` of the variant is appended;
- the root carries the **variant's** `viewBox` and the **specification's**
  pixel `width` and `height`.

That last line is the point. `preserveAspectRatio` then defaults to
`xMidYMid meet`, and SVG's own semantics scale the variant uniformly and centre
it in the canvas. There is no fitting arithmetic in Shaipe to get wrong.

The set of drawn elements is a denylist rather than an allowlist because *that*
set is the closed one: SVG enumerates its drawable elements, while "everything
that only defines" is open-ended. Keeping an element that turns out to be a
definition is harmless; dropping one breaks every fill that referenced it.

## Consequences

- `Format::Svg` costs nothing: an isolated variant already *is* a standalone
  SVG document, so exporting one is skipping the rasterisation step.
- The document is parsed twice per render — once by `roxmltree` to isolate,
  once by `usvg` to draw. For files of this size that is not worth optimising,
  and `Renderer` already builds the font database once for many specifications.
- The isolated document keeps definitions the variant does not use. Pruning
  them is possible and not currently worth the risk of pruning one that was
  needed.
- Variants are not restricted to `<symbol>`. Any element with an id works; if
  it has a `viewBox` that is used, and otherwise the document's is. The
  `<symbol>` convention of ADR-001 is a recommendation the renderer does not
  need to enforce.
