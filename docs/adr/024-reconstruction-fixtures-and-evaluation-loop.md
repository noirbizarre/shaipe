# ADR-024 — Reconstruction is tested with computed fixtures, no model

## Status

Accepted

## Context

The reference-reconstruction epic added six tools and a workflow across
[ADR-018](018-structured-reference-measurement.md) to
[ADR-023](023-tool-contract-conventions.md). Each was tested on its own, with
an image built inside its own test module (four separate, private copies of a
disc generator). Nothing tested them *together*: that a reference can be
analysed, traced, constructed, rendered and compared in order, that the numbers
mean what the descriptions say, or that a loop steered by those numbers
actually converges.

That is the property the epic depends on, and the only test that touches it is
`tests/acp_opencode.rs`, which needs a model, a credential and money, and is
`#[ignore]`d. A regression in a comparison metric would go unnoticed until
someone paid to run it.

[ADR-021](021-explicit-reconstruction-workflow.md) also left a debt: the
strategy thresholds were "a starting judgement call, not a tuned result against
real references", with these fixtures named as the place it would be settled.

## Decision

**Prove everything up to the model, deterministically, in `tests/reconstruction.rs`.**
What a model adds is the decision about what to draw. Replace it with the
fixture's known-good construction and a small policy (`refine` in
`tests/reconstruction/harness.rs`) that steers using only what
`compare_reference` reports — the information the real loop gives a model. If
the numbers were not enough to steer by, the loop would stall and a test would
say so.

**Fixtures are computed, not committed.** `tests/reconstruction/corpus.rs`
draws seven 64-pixel references from integer geometry: a silhouette,
light-on-dark artwork, disconnected components, nested holes, multiple colours,
a mirror-symmetric mark, and a noisy one. Shapes are sampled on an integer
sub-pixel grid, so the bytes are the same on every platform, the pixels a test
depends on are readable beside it, there is no binary to review and no
third-party artwork whose licence could matter. Two controls sit beside the
corpus: an asymmetric mark, so a perfect symmetry score is not a constant, and
nine separate squares, so the region threshold is ever the deciding factor.

**Goldens record facts, not bytes.** Analyses, traces and the improvement
history are `insta` snapshots with floats rounded, in `tests/snapshots/`. A PNG
byte-for-byte comparison would fail on an encoder upgrade that changed no
measurement. This is not hypothetical: a first version also snapshotted the
traced silhouette's SVG text, and macOS fitted one curve segment of the outline
differently from Linux (`c-.01-.57-.01-.57` against `c-.01-1.13-.02-2.27`)
with `vtracer` already pinned by `=`. The same shape, different bytes. The text
is asserted structurally instead, and the geometry is snapshotted rounded to one
decimal, which all three platforms agree on.

**Comparison metrics are held to known behaviour**, on a transparent-background
fixture so every metric can reach a near-perfect score: an exact construction
matches; a translation reports the offset it was moved by (render minus
reference); a half-size construction reports half the width; nothing drawn
overlaps nothing; and the right shape in the wrong colour is a colour error and
not a shape error.

**One test shows iterative improvement.** From a mark that is the wrong size
and in the wrong place, foreground overlap goes 0.00, 0.16, 0.91, 1.00 across
four comparisons — position, then size, then converged — strictly increasing,
asserted to start poor, to take more than one step and to stop before its limit
so the test cannot pass by being trivial. A second test records the visited
phases against `Workflow::for_kind(Reference).phases()`.

## What the fixtures found

The strategy heuristic was wrong, and the fix is part of this change.

`recommend_strategy` counted every entry of `Analysis::dominant_colours`. That
list buckets every opaque pixel, and an anti-aliased edge is made of
intermediate colours, so a flat one-colour mark reported six to eight
"dominant colours", none over 1.4% of the pixels. A plain rounded
square, a disc and a ring were all recommended `construct`; only the
hard-edged fixtures were recommended `trace`. Nothing exercised this
because the previous tests drew hard-edged shapes.

The count now includes only colours covering at least 2% of the opaque pixels
(`SIGNIFICANT_COLOUR_FRACTION`). `MEASURABLE_MAX_REGIONS = 6` and
`MEASURABLE_MAX_COLOURS = 4` are unchanged: with the fringe excluded, they sort
the seven fixtures as intended.

A second finding is about the tracer, not Shaipe: `vtracer` discards any patch
under `filter_speckle * filter_speckle` pixels, 16 by default. The first noisy
fixture used 3x3 specks and the trace quietly cleaned it, which is the opposite
of the failure it was built to show. Its specks are 5x5.

## Consequences

- A regression in `analyze`, `trace`, `compare`, `recommend_strategy` or the
  tool contracts that join them now fails `mise run ci` with no credentials and
  no network. Nothing in the suite calls a model.
- Snapshots must be reviewed with `mise run snapshots` when a measurement
  changes on purpose, and never edited by hand.
- The region threshold is exercised only by the nine-squares control. The other
  fixture that is recommended `construct` is also multi-coloured and would pass
  on colour count alone.
- **Fidelity to the reference is not fidelity to the artwork.** On the noisy
  fixture a trace overlaps the *reference* better than the hand-built mark, and
  overlaps what the artist *meant* far worse, because it reproduces every
  speck. `compare_reference` can only measure the first. An agent that chases
  its overlap alone learns to copy noise, so choosing the strategy is what
  prevents that, and a test records the order reversing rather than hiding it.
- `SIGNIFICANT_COLOUR_FRACTION` is one more judgement call. It sits above the
  largest fringe bucket measured (1.4%), and so also excludes a deliberate
  speck colour at 1.2% in the noisy fixture; that fixture is recommended
  `construct` by its region count and its other colours, so the cutoff is not
  tested there. A very thin-lined logo has proportionally more fringe and
  could push a bucket over it; there is no such fixture yet.
- The fixtures are 64 pixels. They cover the geometry of each case, not the
  scale of a real export, and say nothing about photographs or JPEG
  artefacts, which the tracer and the analysis do not claim to handle. The
  gradient, translucent and stroke fixtures were added later
  ([ADR-025](025-appearance-analysis.md)), and lettering fixtures after those
  ([ADR-028](028-typography-analysis.md)).
- The four private image generators in the module tests are left where they
  are: they are unit fixtures, and moving them is a refactor with no test to
  gain.

## Update: the tool count

Context says the epic "added six tools". Counted across ADR-018 to ADR-023
alone, three are new — `get_reference_analysis`, `compare_reference` and
`get_workflow` — and `get_reference_trace` (from ADR-014) was extended by
ADR-020. Six is right only if the reference tools ADR-014 introduced are
counted with them. Nothing in the decision depends on the number.
