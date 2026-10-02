# ADR-029 — Group regions into components, and report how they sit apart from how they are filled

## Status

Accepted

## Context

The analysis ([ADR-018](018-structured-reference-measurement.md)) reports each
connected region on its own, and typography ([ADR-028](028-typography-analysis.md))
reports the ones that are lettering. A complex logo is neither: a ring with a
dot in it, a wordmark centred beneath it, four equal squares in a row, a few
small ornaments round a disc. Each of those facts is about *several* regions —
that they belong together, that one holds another, that they share a centre
line, that they are spaced alike — and an agent that only has the regions has
to find them by looking, which is exactly the estimate Shaipe exists to replace
([ADR-014](014-deterministic-raster-to-vector-tracing.md)).

Two things have to be kept apart. *How the parts sit* is geometry. *What
they are filled with* is appearance ([ADR-025](025-appearance-analysis.md)). An
agent that is told "these are aligned" should not also be told that they are
the same red, and recolouring a logo must not move anything in the first.

## Decision

**`Analysis` gains `composition`,** a list of components and four lists of
facts about them. It is left out when there are fewer than two components, so
a single shape, a single line of lettering and every earlier golden for one
read exactly as they did.

**A component is a group of regions.** A line of lettering that typography
found is one component, marks included. Every other region is merged with
another when their boxes are disjoint and closer than the tolerance, which is
what the pieces of one anti-aliased shape look like. Boxes that overlap are
never merged: a ring and the disc in its hole are two components, one
*containing* the other, and the relationship is more useful reported than
hidden. A component has a `role`, a `geometry` and an `appearance`.

**The role is size, never meaning.** `primary` is at least half the area of the
largest component that is not lettering; `decorative` is under a twentieth of
it and encloses nothing; `lettering` is a typography line; the rest are
`secondary`. Shaipe does not know what a logo depicts
([ADR-004](004-tools-not-a-model.md)); it knows that a dot beside a disc is
small.

**Contours are open, closed or filled, and exactly so.** A component is
`closed` when its regions enclose at least one hole, `open` when it is
stroke-like and encloses nothing, and `filled` otherwise. `Analysis::holes` is
capped at 32 and attributes a hole by whose bounding box contains it, which is
ambiguous for nested shapes, so composition does not use it: every hole is
attributed to the region it actually touches that has the largest box, and
counted. An open stroke drawn as a closed shape is a different shape, and the
instructions say so.

**Relationships, alignments, spacings and repeats are geometry only.**

- A *relationship* is `contains`, `overlaps`, or a part's nearest neighbour to
  its `right_of` or `below`, each with the gap, the offset of centres and the
  ratio of areas. Neighbours only, so the list grows with the parts and not
  with the pairs of them.
- An *aligned group* is components whose left, centre or right edge (top,
  centre or bottom) agree, as a vertical or horizontal line.
- A *spacing* is a row or column of three or more components, with the mean and
  deviation of the gaps and whether they are `even`.
- A *repeat* is three or more components of one width and one height. Two are
  a pair, and a pair is alike by chance.

**One tolerance serves for all of them:** two pixels, or one and a half percent
of the longer side. The same pixels are never a gap in one place and nothing in
another.

**Geometry and appearance are siblings.** A component carries `geometry` and
`appearance`, the latter the same summary a word carries in typography, and no
other list names a colour. `recolouring_a_composition_changes_its_appearance_and_not_its_geometry`
is what fails if that changes.

**The default is to report nothing.** More than 32 components is a pattern,
and a component of more than 8 pieces is a heap of specks that happen to lie
close. Calling either "the primary artwork" would mislead more than saying
nothing; `noisy` was the first fixture to find the second, and the rule is the
result.

**Instructions say what the tools cannot enforce:** build one element or group
per component, keep ornaments apart from the primary artwork, draw an open
contour as an unclosed stroke and keep a closed one's hole, place parts from the
measured relationships rather than by eye, reuse a repeated part and an even
gap, and read geometry and appearance separately.

## Determinism

Integer boxes, `BTreeMap` and `BTreeSet` throughout, total-order sorts with an id
tie-break, and floats rounded to a tenth (a hundredth for ratios) when the
report is built. The same regions give the same composition on any machine and
in any order they arrive in
(`components_are_numbered_by_size_and_not_by_input_order`).

## Consequences

- `Analysis` has a new optional field. Analyses with two or more components
  gain a `composition` key; nothing else in their goldens moved.
- The thresholds are judgement calls, each a named constant with the reason
  beside it, and each has a control either side of it in
  `src/analysis/composition.rs`. They were set against integer rectangles and
  discs (`tests/reconstruction/corpus.rs`), not logos. Real, anti-aliased,
  touching or rotated artwork is unproven here and will move thresholds.
- `compare_reference` is unchanged. A `composition` diff, matching the
  components of a reference to those of a render, is left to the planning work
  that will consume it.
- Not found: components that touch and so form one region, pieces of one shape
  whose boxes overlap (they stay separate components), groups by similarity
  of appearance, symmetry per component, arrangements on a curve or at an
  angle, and a relationship that is not between neighbours. These are reported
  as nothing, not as a worse guess.
- A component's contour is read from its largest region: a fragment beside it
  is a piece of the same shape, not a line.
- `get_reference_trace` is unchanged. Its paths are colour layers, not
  components.
