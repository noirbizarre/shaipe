# ADR-027 — Compare how a render is filled, reported apart from its geometry

## Status

Accepted

## Context

[ADR-019](019-compare-reference.md) compares a render against a reference by
shape and pixels. A render with the right silhouette and a flat fill where the
reference has a gradient scores well on overlap, offsets and edges, and only
the pixel error hints that something is wrong, without saying what.
[ADR-025](025-appearance-analysis.md) already measures fills, opacity and
strokes per region, for the reference.

## Decision

**`compare_reference` gains an `appearance` field, a diff of the two images'
appearance analyses.** Nothing new is measured: both images go through the same
analysis, so a fill is judged by the same rules on each side.

- Each reference region is matched to the render region sharing most of its
  pixels (ties to the lowest id). No match is reported, not skipped.
- Per match: fill kind, flat colour or stop colours sampled at shared offsets,
  linear axis angle, radial centre and radius, mean opacity, stroke width.
- Canvas-wide transparency, including drawn (interior) translucency, is diffed.
- `findings` holds one sentence per mismatch, naming what differs and what to
  change. It is empty when the fills agree.

Existing fields, thresholds and formulas are untouched, and appearance is never
folded into another score, following ADR-019. The overlay and difference images
are unchanged.

## Consequences

- A gradient reversed, rotated, flattened or made opaque each produce a
  distinct, actionable finding, covered by `tests/reconstruction.rs`.
- The analysis runs twice more per comparison, which is small beside the
  perceptual pass.
- Matching is by pixel overlap: a render that merges or splits regions
  differently is reported as unmatched or as a kind mismatch, which is
  informative but coarse.
- The tolerances (colour 10%, angle 10 degrees, opacity 0.05, ratio 25%) are
  judgements of what is worth an agent's attention; change them with a fixture.
