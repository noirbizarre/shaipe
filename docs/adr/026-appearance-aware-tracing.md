# ADR-026 — Trace a gradient once, and paint it with what was measured

## Status

Accepted

## Context

[ADR-025](025-appearance-analysis.md) measures whether a region is a gradient,
and says so without touching the trace: "a trace cannot help: it only produces
flat fills". That was true and it was a problem. `get_reference_trace` in
`colour` mode clusters by colour and stacks one flat layer per step of
`layer_difference`, so a two-colour ramp came back as ten paths (a 40 pixel
square) and more on a larger one. The fade, which is one fact about one region,
was spread over the geometry as noise. A translucent fill fared worse: a traced
`fill` has no alpha, so half-transparent red came back as full-strength red.

The opposite failure is a trace that becomes a vectorizer: fitting meshes,
diffusion curves, its own idea of a gradient. Tracing is a measurement
primitive ([ADR-014](014-deterministic-raster-to-vector-tracing.md)), and the
agent stays the one that decides what the mark is.

## Decision

**In `colour` mode, a region the analysis measured as a gradient, or as a flat
colour drawn translucent, is lifted out of the colour trace.** Its pixels are
blanked before vtracer sees them, so they are neither fragmented nor spend its
layers. Its silhouette is traced on its own, as a binary mask, into one path
with its holes cut by winding. That path is painted with the gradient or the
opacity the analysis reported.

**Geometry and appearance are two pieces of evidence, joined by `region_id`.**
A lifted path carries `region_id`, which is the id in `analysis.regions` and
`analysis.appearance.regions`, and an `appearance` holding the same `fill` and
`opacity` the analysis reports. Nothing about the outline depends on the fill:
an agent that distrusts the gradient can recolour the path and keep the shape,
and the silhouette alone is still there in `silhouette` mode. The `fill_colour`
of a gradient path is `null`, which is what that field already said for
anything not solid.

**The trace's SVG is the hybrid.** The lifted paths and their `<defs>` are
written into the same document as the remaining flat layers, after them, so it
previews as it will render and can be grafted like any other trace.

**Whatever varies but is not a gradient is reported, not hidden.** A region
the analysis calls `varied` is left as the flat layers a plain trace would give
and listed in `fallbacks` with its fits. It is not an error: touching flat
colours are one region and their layers are the right answer. It is the
explicit statement that no gradient could be vouched for. A gradient the
analysis calls `flat` because it is too subtle (ADR-025) is likewise left
alone.

**On by default, switched off with `appearance: false`.** The alternative,
opt-in, leaves an agent that did not know to ask with a stack of ten colours
for one gradient. With it off, `colour` mode is exactly what it was, which is
also the evidence of what the layers would have been. `silhouette` mode ignores
the flag, as it ignores `max_colors`: it is geometry only by construction.

**No new tool and no new mode.** ADR-025 rejected a separate tool for the same
reason: an agent would have to know to call it. A `TraceMode::Appearance` would
duplicate `colour` for every region that is not lifted.

## What is lifted

- `linear_gradient` and `radial_gradient` regions.
- `flat` regions whose highest interior opacity is below 0.95 and which cover at
  least 64 pixels. A hairline is nothing but anti-aliased edge, whose alpha is
  partial by construction.

`varied` regions, strokes, opaque flat regions and regions beyond the
analysis's 32 are left to the colour trace.

## Consequences

- `vectorize` now imports `analysis`, the arrow the README already drew. It
  uses the analysis's own region map so both tools segment the same way.
- The seven existing `trace_*` goldens are unchanged: the new fields are
  omitted when empty, and nothing in the corpus is a gradient. New goldens cover
  the linear and radial gradients, a translucent fill, and `gradient_badge`, a
  panel with a window cut through it beside a flat bar.
- A lifted outline is the region's mask, anti-aliased rim included, so it
  sits a fraction of a pixel wider than the drawn edge. `compare_reference` is
  where that shows.
- A gradient that passes through the background colour is measured on what
  remains (ADR-025), and is lifted as that.
- A gradient touching a differently coloured flat region is one `varied`
  region, because regions are connected foreground. It is a fallback, not a
  gradient.
- Colour tracing a fully transparent reference used to panic inside vtracer
  (it divides by the number of opaque pixels). It is now an `EmptyTrace`. The
  same guard is what lets a reference that is nothing but a gradient trace to
  just its lifted path, with no colour layers left for vtracer to be given.
- `get_workflow`'s strategy recommendation still ignores appearance.
