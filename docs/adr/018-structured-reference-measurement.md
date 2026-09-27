# ADR-018 — Measure a reference's pixels into facts; do not ask a model to guess them

## Status

Accepted

## Context

`get_reference_trace`/ADR 014 established that an agent looking at a
reference cannot measure it, only describe it: it can name a shape but has no
mechanism for reading an exact curve off the pixels. The same gap exists one
level up, for layout and colour. Asked "where is the mark, roughly how big is
it, what colour is the background, is it symmetric?", a vision model answers
from impression, not from counting pixels — and every one of those questions
has a single, checkable numeric answer sitting in the bytes already.

This is the first item in the reference-reconstruction epic (#12) precisely
because everything downstream needs it: `#5`'s comparison tool needs a
background/foreground split and a bounding box to align a reference against a
render; `#6`'s richer tracing needs the same region primitives, coloured
rather than binary; `#7`'s workflow model assumes an "inspect" phase exists
before "trace" or "construct" are chosen between. None of that has anything
to measure yet.

ADR-004 is why the fix is again not "a better prompt": this is measurement of
the kind `render/` and `vectorize/` already perform, not generation.

## Decision

A new tool, `get_reference_analysis`, and a new engine module,
`src/analysis/`, that measures an attached reference's pixels into a
structured, typed report.

**`src/analysis/mod.rs`** is self-contained and deterministic, mirroring
`vectorize/`'s isolation from everything above it — it knows nothing about
`Project`, tools or MCP. `pub fn analyze(path: &Path, bytes: &[u8]) ->
Result<Analysis>` decodes with the `image` crate (already a dependency; no new
one was needed) and reports:

- **dimensions** and aspect ratio;
- **background**: the image's transparent-pixel fraction, plus a colour
  sampled from the canvas *border* (never the whole image) and how uniformly
  the border agrees with it;
- **dominant colours**: a coarse histogram over opaque pixels, largest share
  first;
- **foreground**: the overall pixel share and bounding box of everything not
  classified as background;
- **regions**: 4-connected foreground components, each with a bounding box,
  pixel area and centroid, largest first;
- **holes**: background components that never touch the canvas border —
  fully enclosed by foreground — each naming the one region that encloses it,
  when exactly one does;
- **symmetry**: left-right and top-bottom mirror scores, computed as an
  intersection-over-union of the foreground mask with its own reflection.

Every one of these is a fact about pixels, never a name. `Region` and `Hole`
carry a bounding box and a centroid, not a guess at what they depict — a
model still decides that a region is a "keyhole"; the tool only tells it
where and how big. The tool's own description says so directly, the same
discipline `get_reference_trace`'s description already practises for its own
"not for photographs" caveat.

No new dependency. Connected-component labelling is a flood fill over an
already-decoded `Vec<bool>` mask; colour and border sampling are `BTreeMap`
histograms. Both are a few dozen lines against a buffer `image` already
produces, and both need `BTreeMap` rather than a hashed map specifically so
iteration order — and therefore every tie-break — is deterministic without
depending on a hasher, the same property invariant 1 (`AGENTS.md`) requires
of a render.

**Background is sampled from the border, deliberately**, rather than guessed
from the whole canvas (a global mode colour, a corner-only sample, or a
clustering step). A reference exported the way a logo normally is — some
margin around the mark — makes the entire border one colour, which is a
strong, cheap, and explainable signal; a histogram over the whole image
cannot tell "large background region" from "large foreground region" without
already knowing which is which. The cost is named openly rather than hidden:
artwork drawn all the way to the canvas edge, with no margin, corrupts the
sample, and both the module doc and the tool's description say so.

**A tool that mutates nothing.** `get_reference_analysis` is a `get_` tool
(ADR 007): it reads a reference and returns numbers, and never changes the
project — the same shape `get_reference_trace` and `get_reference_image`
already have.

**No exposed tuning knobs**, unlike `get_reference_trace`'s `threshold`/
`invert`. `TraceOptions` exists because tracing genuinely is ambiguous
(foreground vs. background is a judgement call a caller sometimes needs to
correct); nothing here has surfaced a case where a caller needs to move the
transparency cutoff, the background-distance tolerance or the histogram
bucket width, so none of the three is a parameter. Extend this the way
`set_` was "anticipated but not used" until it was (ADR-007's own precedent),
if a real caller needs it.

## Alternatives rejected

**Folding this into `vectorize/` instead of a new module.** Both decode a
reference and both separate a foreground from a background, so the overlap
is real. Rejected because the *output* is a different kind of thing — a
structured report of typed facts, not an SVG — and because `#5`'s comparison
tool and `#6`'s richer tracing will each want the background/region
primitives independently of whether tracing happens at all. A shared
decode-and-classify helper can be extracted later if the duplication actually
hurts; two independent modules that happen to both call `image::
load_from_memory` is not that yet.

**Colour-aware, multi-region background separation** (clustering, k-means
over the whole image, a learned model). Rejected for v1 for the same reason
ADR-014 rejected `vtracer`'s colour/photo frontends: a brand mark's
background is overwhelmingly a single flat colour or fully transparent, a
clustering step adds its own tuning surface, and issue #6 is explicitly where
colour-aware region separation belongs once tracing itself needs it.

**Reporting a single opaque "similarity" or "quality" score for the whole
image.** Never considered a fact worth stating — issue #4's own acceptance
criteria and #5's design brief both explicitly reject a single opaque score
in favour of actionable, separately-named measurements, and this ADR follows
that.

**Exposing every constant (transparency threshold, background distance,
histogram bucket width, region/hole caps) as a tool parameter.** Rejected as
premature: every one of ADR-014's exposed knobs answers an ambiguity a real
image actually produced. Nothing here has, yet.

## Verified

Every measurement is covered by a synthetic-image test in
`src/analysis/mod.rs`: a filled disc (one region, high symmetry), a ring (one
region, one hole, correctly attributed), two disjoint differently-sized
squares (two regions, sorted largest first, lower symmetry than the disc), a
transparent-background image and an opaque flat-background one (background
reported differently and correctly in each), and a dominant-colour histogram
capped and sorted by coverage. A margin-less version of the two-squares test
was tried first and confirmed the border-corruption limitation above by
failing exactly as predicted — this is recorded in the test fixture's own
comment rather than only here, so a future reader hits the explanation at the
point they would otherwise be tempted to remove the margin.

## Consequences

- `Error` gains one variant, `AnalysisDecode` (`shaipe::analysis::decode`),
  parallel to `vectorize`'s `Decode` — kept separate because its help text
  names `get_reference_analysis` specifically, and a shared variant would
  have to name both tools or neither.
- `region_count`/`hole_count` report the true total even when `regions`/
  `holes` are capped (32 each) — a caller can tell a clean result from a
  fragmented one (many tiny regions, usually meaning the background was not
  separated cleanly) without the response growing unbounded on a noisy
  image.
- A `Region`'s `id` is assigned after sorting by area, not by flood-fill scan
  order, specifically so it survives truncation: a `Hole::enclosed_by` can
  still name a region that did not make it into the truncated `regions` list.
- This does not itself close any part of the reconstruction loop — `#5`
  (comparison), `#6` (richer tracing) and `#7` (workflow phases) each still
  need their own work, and are expected to reuse the background/region
  vocabulary established here rather than reinvent it.
