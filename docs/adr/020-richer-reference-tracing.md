# ADR-020 — Richer tracing: colour regions, and measuring the SVG a trace produces rather than the pixels behind it

## Status

Accepted

## Context

ADR-014's `get_reference_trace` traces exactly one thing: a single-colour
silhouette, `Clustering::Binary` against a brightness cutoff, holes cut by
winding. It was deliberately narrow — "a brand mark is one colour, the
motivating case was one colour... extend `TraceOptions` if a real project
needs it" — and its own consequences section already named the two gaps this
ADR closes: no colour/photo frontend, and "no preview image travels with the
tool's answer... adding a standalone rasterise path that does not touch a
`Project` is a reasonable follow-up." ADR-018 named the same gap from the
other side, a chapter earlier: "`#6`'s richer tracing needs the same region
primitives, coloured rather than binary."

Issue #6 asks for more than a second frontend: multi-region tracing, colour
extraction, reporting source bounds, and enough metadata that an agent can
reason about a trace without first grafting it into a project and rendering
that. The existing tool answered none of the last three — `path_count` was
`svg.matches("<path").count()`, and nothing else about *what* was traced (how
big, what colour, how many distinct regions before any cap) ever left the
module.

## Decision

Grow `get_reference_trace` and `src/vectorize/`, rather than adding a second
tool: `threshold`/`invert` are already precedent for this tool's schema
growing as real ambiguities turn up (ADR-007's `set_` discipline), and a
second tool would only duplicate the same resolve/refuse plumbing every other
reference tool already shares.

**A second frontend, chosen by a `mode`.** `TraceOptions` gains `mode:
TraceMode` (`Silhouette`, the default and everything ADR-014 already built, or
`Colour`) and `max_colors: Option<usize>`. `Colour` uses
`Config::from_preset(Preset::Poster)`, which vtracer's own defaults already
pair with `Clustering::ColorCluster` — hierarchical colour clustering, each
flat-colour region fitted and painted as its own path. `max_colors` maps
straight onto `Config::max_colors`; nothing else of `Poster`'s surface
(`color_precision`, `layer_difference`, a fixed `palette`) is exposed, the
same restraint ADR-014 applied to `Bw`.

**Colour mode does not flatten transparency.** `Silhouette` composites onto an
opaque backdrop first, because the binary frontend judges darkness from RGB
alone and does not see alpha (ADR-014). The colour-cluster frontend is
different — verified by reading `vtracer`'s own
`frontend/keying.rs`, not assumed: it keys out a background itself once a
sampled row is at least a fifth fully transparent, discarding it as its own
layer. Flattening onto a backdrop before handing it to that frontend would
instead bake the backdrop colour into every anti-aliased edge pixel, the
opposite of what colour tracing is for. So `Colour` mode hands `vtracer` the
decoded RGBA buffer untouched, and `threshold`/`invert` are simply unread in
that branch — not refused if both are set, since a caller reusing the same
call shape for either mode is a convenience, not a mistake.

**Metadata is measured from the SVG `trace` just produced, with `usvg` — not
from `vtracer`'s internal `VectorDoc` IR, and not by re-deriving it from the
source pixels a second time.** `usvg::Tree::from_str` (the same library
[`crate::render`] parses every project SVG with) walks every `Node::Path`,
reading `abs_bounding_box()` and, when the fill is a solid `Paint::Color`,
its hex value. This is more honest than counting pixel components: it
measures the geometry that actually came out, in the same coordinate space
`write_variant`/`write_svg` would receive it in, rather than a raster
approximation of it. `usvg::Options::default()` is enough — a traced SVG never
carries `<text>` or `<image>`, so no font database or resources directory is
ever needed to parse it back — and a parse failure here is treated as
unreachable (`.expect`, with a comment), the same way `Config::build`'s own
unreachable failure modes already are: a failure would be a bug in `trace`,
not something a caller passed in.

New types, deliberately not sharing `crate::analysis::BoundingBox`/`Region`
despite the obvious overlap: `BoundingBox` here is floating-point vector
geometry in the traced SVG's own coordinate space (source-image pixels, since
`vtracer`'s output carries no `viewBox`), where `analysis::BoundingBox` counts
integer raster pixels of a classified mask. They answer different questions
about different things, and forcing them to share a type would hide that.
`TracedPath { id, bounding_box, area, fill_colour }` mirrors
`analysis::Region`'s own discipline instead: `id` assigned after sorting by
area (largest first), stable across truncation — `Traced::paths` caps at
`MAX_TRACED_PATHS` (32, same figure as `analysis::MAX_REGIONS`) while
`Traced::path_count` keeps reporting the true total, so a caller can still
tell a clean trace from a fragmented one on a noisy image.

`TracedPath::area` is the bounding box's own area, not an exact fitted-curve
area — no curve integration is performed. Named honestly as an upper bound in
its own doc comment, rather than pretending to a precision this module does
not compute; a caller that needs the exact figure can rasterise the result
(see below) and count pixels.

**A standalone preview raster.** `vectorize::preview(svg: &str) -> Vec<u8>`
parses the traced SVG with `usvg::Options::default()`, sizes the canvas from
the tree's own `width`/`height` (no caller-chosen canvas — the same restraint
ADR-019 applied to `compare_reference`'s canvas), and rasterises with
`resvg::render` + `Pixmap::encode_png`. It takes no `Project`, `Renderer` or
font database at all: this is the "standalone rasterise path" ADR-014's own
consequences section named, and it costs nothing new — `usvg`/`resvg`/
`tiny-skia` are already crate dependencies via `render/`. `get_reference_trace`
now always attaches one preview image via `ToolOutput::with_image`, the same
"just useful, not a toggle" choice `render_svg`/`get_reference_image` already
make — this is what actually closes issue #6's "a vision-capable agent can
inspect a trace result without first hand-grafting it" criterion.

**Tool output nests the report**, the same shape `get_reference_analysis`
already uses for `Analysis`: `{"src", "resolved", "trace": {...}}` rather than
flattening `svg`/`path_count` at the top level as before. `trace()`'s Rust
signature changes to match: `Result<Traced>` in place of `Result<String>`.

## Alternatives rejected

**`Clustering::Watershed` or `Hierarchical::Cutout` (mosaic compositing).**
Neither has a demonstrated need yet — the same reasoning ADR-014 used to
reject `vtracer`'s colour/photo frontends for v1. `ColorCluster` already
answers "multi-region" and "colour" both; watershed's own tuning knob
(`watershed_detail`) and mosaic's seam-free compositing solve a different
problem (arbitrary photographic segmentation) than a multi-colour mark asks
for. Revisit if a real project needs either.

**A fixed `palette: Vec<Color>`.** `vtracer` supports tracing onto a
caller-chosen set of colours; ADR-014 already reserved this as a future
option and never wired it up. Still nothing has demonstrated a need for it
over auto-clustering plus `max_colors`, so it stays unexposed.

**Reaching into `vtracer`'s `VectorDoc`/`Pipeline::run` for metadata instead of
re-parsing the SVG with `usvg`.** Slightly cheaper (no second parse), and
would expose `vtracer`'s own IR types across a module boundary this crate
does not otherwise depend on outside `vectorize`. Parsing the actual output
with `usvg` instead measures exactly what a caller would get if they rendered
the SVG themselves, and reuses a parser this crate already trusts for every
project document — one fewer thing to keep in sync if `vtracer`'s IR ever
changes shape.

**Dedicated `Hole` objects on traced output, mirroring `analysis::Hole`.**
Holes are already correct *geometry* today — a ring traces to one path whose
fill-rule cuts the inner circle by winding, unchanged by this ADR — just not
reported as separate measured objects. Issue #6 lists "holes and nested
regions" as something to investigate "where reliable," not as an acceptance
criterion; extracting winding-cut sub-paths as distinct measured holes would
need walking `usvg`'s path segments for sub-path structure, a real amount of
additional work for a fact an agent can already see by rendering the preview
image. Deferred until a caller demonstrably needs the numbers rather than the
picture.

**Exact per-path fill area via curve flattening + the shoelace formula.**
Correct, but real additional work (flattening cubic Béziers to a tolerance,
then integrating) for a number issue #6 does not name as an acceptance
criterion — "bounds" and "path count" are what it asks for. `area` stays the
bounding box's own area, documented as an approximation.

## Verified

`src/vectorize/mod.rs`'s existing silhouette tests (determinism, ring →
single path with a hole, transparent-vs-white equivalence, invert, threshold,
blank image, non-image bytes) pass unchanged against the new `Traced` return
shape — the regression coverage issue #6's acceptance criteria ask for.
New tests cover: a ring's reported bounding box against its own known outer
radius, a silhouette path's reported fill colour, colour mode tracing a
two-colour image into multiple paths with distinct fills, colour-mode
determinism, `max_colors` bounding how many distinct fills survive, a
49-square grid proving `path_count` reports the true total while `paths`
stays capped at `MAX_TRACED_PATHS`, and `preview()` rasterising back to the
source image's own dimensions. `src/tools/builtin.rs` covers the tool itself:
the nested `trace` JSON shape, a preview image always present, colour mode
end to end, and an invalid `mode` value refused by name.

## Consequences

- `trace()`'s public signature changes from `Result<String>` to
  `Result<Traced>`; its one caller (`get_reference_trace`) is updated
  alongside it.
- `get_reference_trace`'s JSON output shape changes: `svg`/`path_count` move
  under a new `"trace"` key, and gain siblings (`mode`, `bounding_box`,
  `paths`). A preview image now always accompanies the JSON. Neither is a
  concern for a tool with no external consumers yet beyond this crate's own
  tests and an agent reading its schema fresh each session.
- No new dependency, and no `Cargo.toml` change at all: `usvg`, `resvg` and
  `tiny-skia` are already crate dependencies via `render/`, and `vtracer`'s
  `Poster`/`ColorCluster` path was already compiled in.
- No new `Error` variant: colour mode's failure modes route through the
  existing `Error::Trace` (pipeline failure) and `Error::EmptyTrace` (though
  the latter practically never fires in `Colour` mode, since even a flat
  image traces to one full-canvas region — documented on `trace`'s own doc
  comment rather than left as a surprise).
- Explicit reference reconstruction issue #6 asks for at least one useful
  colour-aware or multi-component tracing path, richer metadata, and
  inspectability without hand-grafting — colour mode, `Traced`'s metadata and
  the standalone preview close all three. Watershed/mosaic frontends, a fixed
  palette, dedicated hole objects and exact fill area remain undone,
  deliberately, per the "Alternatives rejected" section above.
