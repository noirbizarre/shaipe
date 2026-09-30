# ADR-014 — Trace a reference deterministically; do not ask a model to redraw it

## Status

Accepted. Its scope was extended by [ADR-018](018-structured-reference-measurement.md)
to [ADR-026](026-appearance-aware-tracing.md); see
*Update: where this fits now* at the end, which corrects the statements
below that are no longer true.

## Context

An agent given a prompt and a reference image writes `<path d="...">` by
describing what it sees, not by measuring it. It can name the shapes present —
"a hexagon, a shackle, three circuit traces, a keyhole" — and estimate rough
placement, but it has no mechanism for reading an actual corner radius, curve
tangent or organic taper off the pixels. Those numbers come out as "plausible
SVG for this kind of icon," not as a fit to the image.

This surfaced concretely, not hypothetically: an agent working on a project
outside this repository was given a prompt and a reference PNG for a padlock
mark and produced a hand-built approximation — straight-line hexagon, one
plain circular arc for the shackle, solid filled connector dots, a keyhole
built from a circle and a trapezoid. The composition was right; the styling
was not, next to the reference's rounded corners, tapered shackle, hollow ring
nodes and organic teardrop keyhole. The only way to close that gap turned out
to be tracing the same reference through Inkscape's bitmap tracer (`potrace`)
by hand, then cleaning up the result over several turns.

That is exactly the gap this project already names without closing:
`README.md`'s "Not yet" section says plainly, *"Raster → vector... asking a
model to reconstruct an SVG from one is future work"*, and Shaipe's own
`logo.svg` is still, per `PLAN.md`, *"a hand-drawn placeholder"* for the
identical reason — nobody has yet proven an agent can reproduce reference
artwork through this project with acceptable fidelity, because until now
nothing here could measure one.

[ADR-004](004-tools-not-a-model.md) is why the fix is not "write a better
prompt" or "call a second, more capable model": Shaipe gives an agent the
ability to *see* what it drew (`render_svg`, `render_grid`) and to *read* what
a project says about itself; it never generates. A pixel-contour tracer is not
generation — it is measurement, the same deterministic kind of operation
`render/` already performs in the other direction (geometry → pixels). Adding
one is squarely inside what ADR-004 endorses, not an exception to it.

## Decision

A new tool, `get_reference_trace`, and a new engine module, `src/vectorize/`,
that measures an attached reference's pixels into vector paths.

**`src/vectorize/mod.rs`** is self-contained and deterministic, mirroring
`render/`'s isolation from everything above it — it knows nothing about
`Project`, tools or MCP. `pub fn trace(path: &Path, bytes: &[u8], options:
TraceOptions) -> Result<String>` decodes the bytes (the `image` crate, the
same PNG/JPEG/GIF/WEBP/BMP set `get_reference_image` already recognises via
its extension check) and hands the pixels to `vtracer`, a pure-Rust,
MIT/Apache-2.0 framework that performs no file or network I/O of its own — it
only ever consumes an already-decoded buffer and returns a `String`. That is
what makes it fit invariant 1 (`AGENTS.md`): same bytes and same options in,
same bytes out, on any machine, proven by
`tracing_the_same_image_twice_produces_identical_bytes`.

Traced with `vtracer`'s binary/silhouette frontend (`Clustering::Binary`, via
`Config::from_preset(Preset::Bw)`) — never its colour-cluster or watershed
frontends. A brand mark is one colour; those two frontends exist for
photographs and posters, add a clustering step with its own tuning surface,
and are simply the wrong tool for this job. `TraceOptions` exposes exactly two
knobs, both about separating foreground from background on an arbitrary crop:
`threshold` (the binary cutoff) and `invert` (for light artwork on a dark
background). Everything else `vtracer` can do — palettes, `max_colors`,
mosaic/`cutout` compositing, the other two frontends — stays unexposed for
now. `set_` was "anticipated but not used" until `set_palette_colour` needed
it ([ADR-007](007-tool-names.md)); this follows the same discipline rather
than exposing a large surface speculatively.

A transparent pixel is composited onto an opaque backdrop before tracing —
white by default, black when `invert` is set — because `vtracer`'s binary
frontend judges darkness from RGB alone and does not see alpha; an
un-flattened transparent pixel would be classified by whatever colour its RGB
channels happen to hold underneath the transparency, not by what it looks
like. This was found by reading `vtracer`'s own frontend source
(`frontend/binary.rs`), not assumed — the same discipline `AGENTS.md` already
asks for ("Verify the SVG stack; do not assume it").

**`get_reference_trace`** is a `get_` tool ([ADR-007](007-tool-names.md)): it
reads a reference and returns markup, and never changes the project. It
resolves `src` exactly as `get_reference_image` does, reads the file, calls
`vectorize::trace`, and returns the resulting SVG text plus a path count. It
does **not** graft the result into a variant itself — the caller reviews it,
decides how to recolour it (the traced path carries no `fill` beyond whatever
solid colour the frontend assigned) and where it belongs, and lands it with
`write_variant`/`write_svg`, the same hand-splice motion already used for any
other markup. A tool that tried to merge automatically would be doing the
judgement call an agent made by hand for the case that motivated this ADR:
dropping the traced result's own coordinate-specific gradient and applying
the project's existing `gradPrimary` instead.

## Alternatives rejected

**A C `potrace` binding.** The literal tool used by hand for the motivating
case, and the more established algorithm. Rejected for the same reason
`ratatui-image`'s `chafa-dyn` feature is off by default
(`Cargo.toml`): it needs a system library through FFI, which no CI runner is
guaranteed to have and which reintroduces exactly the kind of machine
dependency invariant 1 exists to rule out elsewhere. `vtracer` is pure Rust,
compiles to `wasm32-unknown-unknown` by its own account, and needs nothing
beyond `cargo build`.

**Colour or photo tracing.** `vtracer` can do both, and it would have been
almost free to expose given the dependency is already here. Rejected for v1:
a brand mark is one colour, the motivating case was one colour, and every
additional frontend is an additional thing to prove deterministic and an
additional part of the schema a model has to read before it can be sure which
knob it wants. Extend `TraceOptions` if a real project needs it.

**Grafting the traced result into the project automatically.** Tempting,
since the tool already knows which reference was traced. Rejected because
"which variant, if any, does this become" and "what colour replaces the
traced placeholder" are exactly the calls Shaipe's tools never make for the
caller elsewhere — `write_variant` validates a fragment the agent already
composed, it does not compose one. A tracer that guessed would be the one
place in the registry doing more than "a name and a shape over an operation
the library already performs" (`src/tools/builtin.rs`'s own module doc).

## Verified

Traced the actual crop `lock.svg` was hand-traced from (the clean "Logo mark
(SVG)" instance in the motivating project's reference mockup) through this
tool. At the default threshold it produced dozens of small, meaningless
fragments rather than the mark — not a bug: the mark's green-to-blue gradient
sits at roughly 130-140 average RGB intensity, which reads as *lighter* than
the default 128 cutoff tuned for genuinely dark artwork, so almost none of it
counted as foreground. Raising `threshold` to 210 produced two paths — the
hexagon/shackle/traces/nodes silhouette and the keyhole — matching the
hand-traced `lock.svg` closely: rounded corners, a tapered shackle, hollow
ring nodes, an organic keyhole. The tool's description now says so directly,
so a caller that hits the same fragmented result knows what changed rather
than guessing.

## Consequences

- `Cargo.toml` gains two dependencies: `vtracer` (pinned to an exact pre-1.0
  alpha version — its changelog needs re-reading before it is ever bumped) and
  codec features on the existing `image` dependency (`png`, `jpeg`, `gif`,
  `webp`, `bmp` — the same set `get_reference_image` advertises), which had
  been deliberately codec-free until now because nothing decoded a file rather
  than a `tiny-skia` pixmap.
- A tracer only ever produces a *candidate*. It can get the geometry right and
  still hand back the wrong number of paths for the intended variant
  structure, an unwanted intermediate colour, or a silhouette that needs
  `invert`/`threshold` adjusted — the caller is still expected to look at the
  result (`render_svg` on whatever it becomes, once landed) before treating it
  as final.
- The dogfooding gap this ADR responds to (Shaipe's own `logo.svg` is still a
  hand-drawn placeholder) is not itself closed by this ADR — regenerating that
  artwork through `get_reference_trace` is the natural next step, and remains
  a separate, undone item.
- No preview image travels with the tool's answer in this first version,
  unlike `render_svg`/`render_grid`. The traced SVG can be looked at by landing
  it in a scratch variant and rendering that, the same two-step motion any
  other hand-composed fragment already needs; adding a standalone rasterise
  path that does not touch a `Project` is a reasonable follow-up, not a
  requirement of shipping this.

## Update: where this fits now

This record was written when `get_reference_trace` was the only reference tool.
The decision stands; its surroundings grew. Nothing above was edited, so three
statements in it are now history rather than description:

- *"A brand mark is one colour"* and *"never its colour-cluster frontend"* —
  `get_reference_trace` has a `colour` mode beside the silhouette it was
  written for ([ADR-020](020-richer-reference-tracing.md)). Silhouette is still
  the default.
- *"No preview image travels with the tool's answer"* — one always does, and
  the answer carries an overall bounding box and per-path metadata
  ([ADR-020](020-richer-reference-tracing.md)).
- *"Nothing here could measure one"* — a reference can now be measured
  ([ADR-018](018-structured-reference-measurement.md)), a render compared to it
  ([ADR-019](019-compare-reference.md)), and the loop proven without a model
  ([ADR-024](024-reconstruction-fixtures-and-evaluation-loop.md)).

**Tracing is measurement, not generation.** This is the argument the whole
reconstruction workflow rests on, so it is restated where the workflow is
described. A tracer reads pixels and returns geometry the pixels imply; the same
bytes and options give the same output. What a model does — decide what the
mark *is*, whether the trace is faithful, what to keep, recolour or redraw — is
not something Shaipe does for it. That is why the tool returns a *candidate*
and never grafts it into a variant: it would be making the model's decision.

**The three workflows.** [ADR-021](021-explicit-reconstruction-workflow.md)
names them and the agent chooses; Shaipe never infers one from what is attached.

| Workflow | When | Phases |
|---|---|---|
| `from_scratch` | No reference. There is nothing to measure or compare against. | `construct, render, inspect, refine, validate` |
| `reference` | A `source` reference is to be reproduced. | `inspect, analyse, choose_strategy, construct, render, compare, refine, validate` |
| `hybrid` | A reference exists, but exact reproduction is not the goal. | the same as `reference`; `get_workflow` always recommends mixing a trace with hand-built geometry |

**What reference analysis is for.** `get_reference_analysis`
([ADR-018](018-structured-reference-measurement.md)) turns pixels into facts —
dimensions, background and foreground, dominant colours, regions, holes,
symmetry — before anyone chooses how to draw. Its measurements are the only
input to `get_workflow`'s trace-or-construct recommendation, which is why a
reference is analysed before it is traced: a photograph traces into noise, and
the region and colour counts say so before that is discovered.

**The loop.** Construct or trace, `write_svg`/`write_variant`, `render_svg`,
`compare_reference`, refine. `compare_reference` renders the variant at the
reference's own pixel size ([ADR-019](019-compare-reference.md)) and reports
overlap, bounding-box and centroid offsets, area difference and pixel error, so
the agent steers by numbers instead of an impression. A write that is accepted
is a valid document, not a finished one, and its result says so
([ADR-023](023-tool-contract-conventions.md)).

**Who owns what.**

| Deterministic Rust | Model reasoning |
|---|---|
| Decoding and measuring a reference (`analysis`) | Deciding what the mark depicts |
| Tracing pixels into paths (`vectorize`) | Whether to trace, construct or mix, given the measurements |
| Rendering and comparing (`render`, `compare`) | Reading the overlay and heatmap, and what to change next |
| The phase list and the strategy recommendation (`workflow`) | Whether to follow them — nothing enforces it |
| Validating a write, never saving it (`tools`) | The SVG itself: structure, palette use, what to redraw |

The left column is reproducible byte for byte and tested without a model
([ADR-024](024-reconstruction-fixtures-and-evaluation-loop.md)). Nothing in the
right column is tested without one, and the tests that try are `#[ignore]`d.

**SVG stays the source of truth.** A trace, a comparison and a workflow are
all *derived*: none is written into the project. The reference is a path in the
metadata ([ADR-001](001-svg-as-source-of-truth.md)); the work is an SVG the
agent lands with `write_variant`/`write_svg`, held in memory until the user
saves. No workflow state is persisted, so opening and saving a project still
changes nothing (invariant 2), and a project reconstructed from a reference is
an ordinary editable SVG, not a wrapper around the raster.

**Three texts, three owners.** The project's `<shaipe:prompt>` is the user's
brief, committed with the document. The turn is that prompt wrapped by
`instruct()`, sent when the user presses `a`, and it carries no workflow, only a
pointer to `get_references` and `get_workflow` when references are attached.
Shaipe's own instructions for using its tools are generated in
`src/workflow/instructions.rs` and sent as the MCP server's `instructions`
([ADR-022](022-reconstruction-instructions.md)). They never enter the prompt.

**What this update does not change.** The workspace's controls, panes and
prompts are as before. Any user-facing change to how a reconstruction is
started, watched or reviewed belongs to the UX epic that follows, not to this
one.

**Further statements the sections above no longer match.** `trace` returns
`Result<Traced>`, not `Result<String>`: the SVG travels with the measurements
made from it ([ADR-020](020-richer-reference-tracing.md)). `TraceOptions` has
four fields — `mode`, `threshold`, `invert` and `max_colors` — not "exactly two
knobs", and `max_colors` is exposed. The README's "Not yet" wording this ADR
quotes as its Context (that asking a model to reconstruct an SVG from a
reference is future work) is gone from the README.
