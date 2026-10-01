# ADR-028 — Measure lettering as geometry, and let the agent read it and admit what it cannot

## Status

Accepted

## Context

A logo with a wordmark is where reconstruction fails in a particular way. The
analysis ([ADR-018](018-structured-reference-measurement.md)) reports each
connected region on its own, so a word is a heap of unrelated boxes, one per
letter, capped at 32. The trace ([ADR-014](014-deterministic-raster-to-vector-tracing.md))
turns those letters into outlines that cannot be edited as words. An agent that
takes them at face value draws every letter as a path: the text can no longer
be changed, it never lines up with its own baseline, and the font is whatever
the curves happened to be.

Two things have to be kept apart. *Where lettering is, and how it is set*, is
geometry: letters share a baseline, are about as tall as each other and are
spaced alike. *What it says, and which font it is*, is reading, and Shaipe has
no OCR and never will ([ADR-004](004-tools-not-a-model.md)); the agent already
has eyes. The risk on that side is not that the agent cannot read, it is that
it reads confidently and wrongly, and the wrong word is then drawn into a file
that looks finished.

## Decision

**Lettering is a candidate with a confidence, measured from boxes alone.**
`Analysis` gains `typography`, a list of text lines. A line has the regions in
it (all of them, not only the 32 reported), a bounding box, an orientation, a
baseline and its slope, the height of the tall letters and, when the case is
mixed, of the short ones, letter spacing, the words, and a confidence. It
cannot say what the letters spell. The field is left out when nothing is found,
so a reference without text reads exactly as it did before, and every earlier
golden is unchanged.

**The default is to report nothing.** A line needs at least three letters,
comparable in height, chained along one axis, with most of them on one
baseline, and enough letters of enough different widths to score 0.6. A row of
shapes all of one size is not lettering whatever the spacing, because that is
what a dotted pattern, a bar chart's ticks and a texture's specks look like;
`noisy` was the first fixture to find this, and the rule is the result. Slabs
and slivers (letters are neither) and more than 512 regions are never
considered.

**Marks belong to a line without being letters.** The dot over an *i*, an
accent and a full stop are small regions near a letter; they are attached to
the line and counted apart (`mark_count`), so they neither inflate the letter
count nor make a letter look short.

**Descenders and punctuation are counted, not hidden.** Letters off the
baseline are reported in `off_baseline_count` and left out of the heights, so a
*g* does not make its line look tall.

**Appearance stays per group.** Each line and each word carries its own
`appearance`: uniform or not, the fill kinds present, and the colour (or the
colours, when it is not uniform). A wordmark and the symbol beside it are not
one assignment, and neither are two words of different colours. Every member is
measured, not only the reported regions.

**Horizontal and vertical text share one detector,** run in a frame where *a*
runs along the line and *c* across it. Vertical text finds its shared edge
among left, right and centre. Consecutive horizontal lines are grouped into
blocks, with their alignment (`left`, `centre`, `right`, `justified`, `none`),
line pitch and the evenness of that pitch.

**The comparison reports lettering line against line.** `compare_reference`
gains `typography`: for each reference line, the matching render line (by box
overlap), the baseline offset, the ratio of letter heights and of lengths, the
difference in letters, words and letter spacing, and in colour, each with a
`findings` sentence saying what to change. Nothing is folded into a score
([ADR-027](027-appearance-aware-comparison.md)).

**Doubt is written into the artwork.** The agent reads the text and sets it as
a `<text>` element in a declared font. How sure it is goes on the element as
two attributes, `data-shaipe-confidence` (`high`, `medium` or `low`) and
`data-shaipe-note`. No second source of truth is introduced
([ADR-001](001-svg-as-source-of-truth.md)), nothing changes in the
project format, and `usvg` ignores the attributes, so rendering is untouched
(`text_carrying_shaipe_attributes_renders_the_same_as_without_them`).
`compare_reference` lists the variant's `<text>` as `declared_text`, and says
so when the reference has lettering and the variant has no `<text>`, when text
carries no confidence, when it is low, and when it names no family, which would
render in whatever the machine has.

**Looking harder is a parameter, not a tool.** `get_reference_image` takes an
optional `x`, `y`, `width`, `height` and `scale`, and returns that area enlarged
as PNG. A text line's `bounding_box` is exactly those four numbers. All four or
none; a mistyped or out-of-bounds area is refused with the image's size.

**Instructions say what the tools cannot enforce:** read the text yourself,
enlarge it first, never invent it, mark doubt, prefer `<text>` in a declared
font, size it from the measured height, keep each word's fill its own.

## Determinism

Integer boxes, total-order sorts, a median-based slope estimate, and floats
rounded to a tenth in the report. The same regions give the same lines on any
machine and in any order they arrive in
(`detection_is_independent_of_the_order_regions_arrive_in`).

## Consequences

- `Analysis` has a new optional field, and `Comparison` a new optional section.
  Neither appears for artwork without lettering.
- The thresholds are judgement calls, each a named constant with the reason
  beside it, and each has a control either side of it in
  `src/analysis/typography.rs`. They were set against blocks, not glyphs: the
  fixtures (`tests/reconstruction/corpus.rs`) are integer rectangles standing
  in for letters, because a glyph rasteriser would make the goldens depend on
  anti-aliasing a platform chose, and what is measured is layout. Real,
  anti-aliased, touching or script lettering is unproven here and will move
  thresholds.
- Not found: text knocked out of a shape (its letters are holes, not
  regions), letters that touch and so form one region, lettering on a curve,
  and letters turned by more than a gentle slope. These are reported as
  nothing, not as a worse guess.
- `get_reference_trace` is unchanged. Its paths are colour layers, not letters,
  so tagging them as text by region would have been a claim the trace cannot
  back; its description points at `typography` instead.
- `required_families` still ignores weight and style. A bold run with only a
  regular face declared renders, but not bold; that is a font question and is
  left to a font ADR.
- `vision` and `settings` are untouched. Shaipe still names no font and reads
  no text.
- The comparison cannot tell that the *text* is right, only that the lettering
  it renders stands where the reference's does. Only the agent, looking, can
  say the letters are the right letters.
