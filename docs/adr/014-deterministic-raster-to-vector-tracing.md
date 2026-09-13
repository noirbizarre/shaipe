# ADR-014 — Trace a reference deterministically; do not ask a model to redraw it

## Status

Accepted

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
