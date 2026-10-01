# ADR-025 — Measure how a region is filled, and say so when it cannot be told

## Status

Accepted

## Context

[ADR-018](018-structured-reference-measurement.md) measures where a reference's
regions are and which colours it contains. Both are answers about *flat*
colour. A region that fades from red to blue is reported as a handful of
colour buckets; a translucent fill is reported at full strength, because a
pixel only counted as opaque or not; and a thin ring is a region like any
other. [ADR-024](024-reconstruction-fixtures-and-evaluation-loop.md) said it
outright: the fixtures "say nothing about photographs, gradients or JPEG
artefacts".

An agent that takes a gradient for a flat colour draws the wrong thing, and a
trace cannot help: it only produces flat fills (ADR-014). The opposite mistake
is as costly. A detector that calls any colour variation a gradient sends the
agent to reconstruct noise as a smooth ramp.

## Decision

**`Analysis` gains an `appearance` field, so one call answers it.** It holds an
alpha summary for the image and, for each reported region, a `fill`, an
`opacity` and a `stroke` that is `null` when the region is not stroke-like. A separate tool was considered and
rejected: the agent would have to know to call it, and a flat-versus-gradient
answer belongs beside the region it is about.

**A fill is `flat`, `linear_gradient`, `radial_gradient` or `varied`.** `varied`
is the default and the honest answer for noise, texture, stripes, a photograph,
a hard step between two colours and a ramp too noisy to trust. It carries both
fits so "almost a gradient" is visible rather than rounded away. A gradient
is claimed only when a model explains at least 90% of the colour variance
*and* the fitted profile is a smooth ramp:

- a one-dimensional profile fits a step or stripes perfectly, so the fit alone
  is not evidence. The profile must have no single step carrying half its
  change, at least a third of its bin-to-bin changes must be real ramp
  changes, and it must reduce to at most six stops;
- a region whose colour varies by less than about 6 levels is `flat`. A
  gradient subtler than that reads as flat, on purpose;
- a radial model must beat the linear one by a margin, since a radial
  gradient centred at an edge mimics a linear ramp on an elongated shape.

**Alpha is a fourth channel, kept apart from colour.** A fade of one colour into
transparency is a gradient whose stops share their colour and differ in
`opacity`. Each region reports its opacity apart from its fill, and the alpha
summary separates transparent, translucent and opaque pixels. It also reports
`interior_translucent_fraction`, the translucent pixels whose neighbours are
all translucent, which is translucency that was drawn rather than an
anti-aliased edge.

**Dominant colours weigh each pixel by its alpha.** Counting a half-transparent
pixel whole reported a translucent fill as a full-strength colour. For an opaque
image nothing changes.

**A stroke is a candidate from geometry alone.** A region much longer than it
is wide, of even width, is reported with its width, length and whether it
encloses a hole. It must be at least 12 pixels, at least 4 times longer than
wide and of at least 0.6 width uniformity (`elongation`, `width_uniformity`),
with a `confidence` that rises to full at 12 times. A filled ring and a stroked circle are the same pixels, so the
report says what the geometry looks like and leaves what to draw to the agent.

**Only the interior is sampled.** A region's anti-aliased rim is eroded away
before fitting, so an edge is never read as a gradient. The extent of the axis
or radius still comes from the whole region, so the endpoints are read off the
fitted line at the region's edges.

## Determinism

Samples are thinned by a fixed stride in scan order, ties are broken by scan
order, and directions come from `sqrt` on an eigenvector, which is correctly
rounded, rather than from trigonometry. The one `atan2`, for the reported
angle, is rounded to a tenth of a degree. Snapshots round floats to a fixed
number of places.

## Consequences

- Every `analysis_*` golden gains an `appearance` block.
- The thresholds are judgement calls tuned against computed fixtures with
  known answers, and negative controls (`hard_step`, `speckled_fill`, and every
  corpus fixture) that make a detector which calls everything a gradient fail.
- A gradient that passes through the background colour loses that part of the
  region, because the background is one colour sampled from the border.
  The gradient is then measured on what remains.
- A region under 64 interior pixels is reported `flat`; a gradient cannot be
  claimed from so little.
- Only the largest 32 regions are described, as for `regions`.
