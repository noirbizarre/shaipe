# ADR-019 — Compare a reference and a render on a canvas the tool chooses, never the model

## Status

Accepted

## Context

ADR-018 gave a reference a background/foreground split, a bounding box, and
connected regions — and said so explicitly at the time: "`#5`'s comparison
tool needs a background/foreground split and a bounding box to align a
reference against a render... None of that has anything to measure yet." This
ADR is that comparison tool.

Issue #5's own brief is specific about what it must *not* do: no single
opaque "similarity" score, reference loading kept separate from rendering,
and the tool — not the model — must establish a consistent canvas before any
pixel-level number means anything. A vision model asked "how close is this
render to the reference" can offer an impression, but cannot report an
intersection-over-union, a centroid offset in pixels, or a mean squared
error — the same gap ADR-014 and ADR-018 each closed one level down.

## Decision

A new tool, `compare_reference`, and a new module, `src/compare/`, together
with a refactor of `src/analysis/`'s internals.

**The canvas is always the reference's own pixel dimensions.** No
`width`/`height` parameter is exposed. `compare_reference` renders the
requested variant at exactly the reference's width × height before comparing
— never the reverse, and never a third size either side had to agree on.
`render/`'s own renderer already fits content aspect-preserving and centred
onto any canvas (see `RenderOptions` and the renderer's own tests), so this
means the reference is never resampled at all: zero interpolation, one fewer
filter choice to get wrong, and one fewer parameter for a model to invent a
value for. This is the same restraint ADR-018 applied to its own tool ("no
exposed tuning knobs... until a real caller needs one") — a caller-chosen
canvas can be added later if a real need for it turns up; none has yet.

**`src/analysis/`'s decode-and-classify step is extracted and reused.**
`analyze()` used to decode an image and separate its foreground from its
background inline. That work — a `Classified` struct and a `classify`
function — is now `pub(crate)` and shared: `analyze()` builds its report from
it exactly as before (same output, proven by the existing exhaustive tests
continuing to pass unchanged), and `compare::compare()` calls it once per
image instead of reimplementing background sampling and foreground
classification a second time. `whole_bounding_box` (already existed) and a
new `whole_centroid` are promoted to `pub(crate)` for the same reason: both
modules measure a mask's shape the same way.

**Metrics are sourced by how trivial they are to get right.** Foreground and
edge mask overlap, bounding-box and centroid offsets, area difference, and
whole-canvas pixel error (changed-pixel fraction, MAE, MSE, RMSE, max error,
per-channel error) are hand-rolled arithmetic over two already-classified
images — the same kind of thing `analysis` and `vectorize` already do
themselves, and simple enough that reimplementing them is the safer choice.
Perceptual similarity is not: SSIM's maths is easy to get subtly wrong in a
way that only shows up on a specific input, so `compare` borrows
`image-compare` (MIT, already builds on the `image` crate this codebase
already depends on) for `rgba_hybrid_compare`, and reuses its own per-pixel
diff image — `Similarity::image.to_color_map()` — as the difference heatmap,
rather than building a second one by hand.

**Edge/contour difference is a Sobel gradient magnitude, thresholded into a
mask, compared the same way the foreground mask is.** This answers a
different question than foreground overlap: two silhouettes can share almost
every foreground pixel while their outlines sit over different sub-shapes
(a ring vs. a filled disc of the same outer radius), or the reverse for a
mark translated by a few pixels. Treating an edge mask as just another mask
to overlap, rather than inventing a separate contour-matching algorithm,
keeps this at the same level of ambition as everything else here:
measurement, not shape recognition.

**No single score, anywhere in the report.** `Comparison`'s fields are each
named for one fact — `foreground.intersection_over_union`,
`bounding_box.offset`, `pixel_error.mean_absolute_error`,
`perceptual_similarity.score` — and none of them is combined into an overall
verdict. `perceptual_similarity.score` is itself a single number, but it is
one named, specific measurement (an SSIM-based hybrid comparison) among
several, not a stand-in for all of them.

**Four images, in a fixed order.** The reference (as attached, not
resampled), the current render, an overlay (reference-only foreground in one
colour, render-only in another, agreement in a third, over a transparent
canvas) and the difference heatmap — matching issue #5's own "Output" list
verbatim.

## Alternatives rejected

**A caller-chosen canvas, with the reference resampled onto it.** This is
strictly more general — it would let `compare_reference` check a render
against a reference at, say, favicon size — but it adds an interpolation
filter choice (nearest? bilinear? Lanczos?) and a second source of
non-determinism risk for a need nobody has demonstrated yet. Revisit if a
real caller needs to compare at a size other than the reference's own.

**Hand-rolling SSIM.** Rejected on the same grounds ADR-014 accepted
`vtracer` rather than hand-rolling a raster-to-vector tracer: this is
numerically fiddly code with failure modes that only show up on specific
inputs, a well-tested crate already exists and already depends on `image`,
and the project's own guidance for this feature was explicit — borrow the
perceptual metric, keep the simple ones under direct control.

**Folding edge/contour comparison into a general "shape descriptor"
library**, or computing actual vectorised contours (via `vectorize`) and
comparing curves. Rejected for v1 as more machinery than the issue asks for:
a thresholded gradient-magnitude mask compared the same way as the
foreground mask answers "do the outlines roughly agree" without building a
second geometry pipeline. `vectorize`'s own tracer remains the tool for
"give me an editable curve", not for measuring how two already-known shapes
differ.

**A single opaque similarity/quality score for the whole comparison.**
Explicitly rejected by issue #5's own brief, and by ADR-018's identical
stance on `get_reference_analysis`. Each signal stays named.

## Verified

`src/compare/mod.rs` covers: comparing an image against itself (perfect
overlap, zero pixel error, near-maximum perceptual similarity), the same
comparison run twice (determinism), a shape translated by a known pixel
offset (the reported centroid offset matches it), a shape resized by a known
factor (the reported bounding-box ratio matches it), two materially
different shapes (low overlap, low perceptual similarity, nonzero pixel
error), a dimension mismatch refused with a typed error rather than a panic,
and a 512×512 "typical logo-sized" pair completing well inside a wall-clock
budget. `src/tools/builtin.rs` covers the tool itself: four images in the
documented order with the right labels and MIME types, an unattached `src`
refused with the list of what is attached, and an unknown `variant` refused
the same way `render_svg`'s own equivalent case is. `analysis`'s own
pre-existing tests are the regression net for the `classify` extraction:
`analyze()`'s output is unchanged, asserted exactly as it was before this ADR.

## Consequences

- `Error` gains two variants: `CompareDimensionMismatch`
  (`shaipe::compare::dimension_mismatch`), unreachable through the tool
  itself but kept typed rather than an assertion because `compare::compare`
  is reachable from more than one call site within the crate; and
  `PerceptualSimilarity` (`shaipe::compare::perceptual_similarity`), wrapping
  whatever `image-compare` itself reports.
- `Error::AnalysisDecode`'s help text no longer names `get_reference_analysis`
  specifically, now that `classify` — and therefore a decode failure — is
  shared by that tool and `compare_reference`.
- A new dependency, `image-compare = "0.5"`, alongside the existing `image`
  dependency it builds on. `vtracer` and `image-compare` are now both
  third-party image-processing crates this codebase leans on for something
  it deliberately does not hand-roll — a pattern, not an exception.
- `analysis::Classified`, `analysis::classify`, `analysis::whole_bounding_box`
  and `analysis::whole_centroid` are now `pub(crate)` shared primitives. A
  future richer-tracing tool (#6) can reuse them the same way, exactly as
  ADR-018 anticipated.
