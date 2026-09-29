# ADR-021 — Name the reconstruction workflow's phases; do not track whether they happened

## Status

Accepted

## Context

ADR-018 named the gap this ADR closes before any of `#4`–`#6` existed to fill
it: "`#7`'s workflow model assumes an 'inspect' phase exists before 'trace' or
'construct' are chosen between." Nothing in the codebase named that sequence
anywhere. An agent with `get_reference_analysis`, `get_reference_trace` and
`compare_reference` in front of it, and no stated order, has to invent one —
and a general-purpose agent inventing the order for reference reconstruction
is exactly the premise issue `#7` (and the epic, `#12`) exists to replace with
something explicit.

Three different situations do not share one sequence:

- **`from_scratch`** — no reference exists. There is nothing to measure and
  nothing to compare a render against, so forcing an `analyse` or `compare`
  step onto this case would be asking for evidence that cannot exist.
- **`reference`** — a `Source` reference exists (see
  `src/project/reference.rs`'s `ReferenceKind`) and the point is to reproduce
  it. `get_reference_analysis`/`get_reference_trace` exist precisely because a
  vision model can describe a shape but cannot measure it (ADR-014, ADR-018);
  a workflow that lets an agent skip straight to hand-describing geometry from
  the image wastes the tools `#4`–`#6` already built.
- **`hybrid`** — a reference exists, but reproducing it exactly is not the
  goal. Tracing and hand-construction are both expected to land in the same
  artwork.

Issue `#7`'s own "important distinction" is the hard constraint on the shape
of the fix: *"This is workflow guidance, not a replacement for the agent's
reasoning. The agent still chooses how to construct the SVG, but Shaipe
provides the intended sequence and the available evidence at each phase."*
Two designs were considered against that constraint before this one:

- **Self-reported phase completion** — a tool taking `{"kind", "done":
  ["inspect", "analyse", ...]}` and computing whatever remains. Rejected: it
  asks the agent to assert its own compliance, which is exactly the kind of
  fact Shaipe otherwise refuses to take on faith (ADR-018's whole point is
  that a model's report of a measurement is not the measurement).
- **Persisted, enforced workflow state on the project** — a `<shaipe:workflow>`
  metadata element, updated as tools are called, refusing `write_svg` before
  `compare_reference` has run for reference work. Rejected on two grounds:
  it is a project-format change (invariant 10, schema versioning) for a
  bigger question than this issue asks, and it contradicts the "guidance, not
  a replacement for the agent's reasoning" constraint directly — an agent
  that reasonably decides to skip straight to construction should be able to,
  the same way `Policy::Guarded` cannot actually stop an agent's own tools
  (ADR-012/013's own lesson: reasoning about one layer and concluding about
  the system is the mistake to avoid).

## Decision

**A new, self-contained module, `src/workflow/`,** mirroring `src/analysis/`
and `src/vectorize/`'s isolation: plain, `Serialize`-only Rust types, no
knowledge of `Project`, `Tool`, MCP, ACP or the TUI.

**`WorkflowKind`** (`FromScratch`, `Reference`, `Hybrid`) is chosen by the
agent, never inferred from a project's attached references — the same
restraint `TraceMode`/`compare_reference` take with their own options, and
consistent with "the agent still chooses."

**`WorkflowKind::phases()`** returns a fixed, ordered `&'static [Phase]` per
kind:

- `from_scratch`: `construct, render, inspect, refine, validate` — no
  `analyse`, `choose_strategy` or `compare` exist for this kind at all, so
  tracing is structurally never forced onto it, and `validate` means checking
  the render itself.
- `reference`/`hybrid` (identical sequence): `inspect, analyse,
  choose_strategy, construct, render, compare, refine, validate`.

This is what makes "the reference workflow explicitly requires visual
validation before completion" (the issue's own acceptance criterion) a fact
about the type rather than a convention written in a tool description
somewhere: a `reference`/`hybrid` phase list cannot name `validate` without
having already named `compare` earlier in the same fixed slice. There is
nothing to falsify, because nothing is asserted about what actually happened
— only what the ordered phases *are* for that kind, which is exactly the
"available evidence at each phase" issue `#7` asks for, not a completion
tracker.

**`recommend_strategy(kind, &analysis::Analysis) -> StrategyRecommendation`**
answers `choose_strategy` for `reference`/`hybrid` work, reusing
`get_reference_analysis`'s own measurements (`region_count`,
`dominant_colours.len()`, `hole_count`) rather than inventing a second set of
facts:

- `reference` recommends `Trace` when the geometry looks clean enough —
  `region_count <= 6` and `dominant_colour_count <= 4`, a starting judgement
  call in the same spirit as `analysis`'s own thresholds
  (`BACKGROUND_DISTANCE_THRESHOLD`, `COLOUR_BUCKET_SIZE`) — and `Construct`
  otherwise, e.g. a photograph or a busy multi-region image where tracing
  would reproduce noise rather than a clean mark.
- `hybrid` always recommends `Hybrid`, regardless of measurability: choosing
  that kind is itself the decision to mix a trace with hand-constructed
  geometry, and the measurements travel alongside the recommendation as
  context rather than a variable that changes it.

**One new tool, `get_workflow`** (`src/tools/builtin.rs`), read-only, so the
model is reachable through the existing MCP/ACP path today — the issue's own
"consumable... without requiring UX changes" criterion — without any TUI
change, which the epic (`#12`) explicitly rules out for this stage. Input:
`{"kind" (required), "src" (optional)}`; `src` follows the same
resolve-before-read, refuse-by-name pattern every other reference tool uses
(`get_reference_analysis`, `compare_reference`). Giving `src` with
`kind: "from_scratch"` is refused rather than silently ignored — there is no
`choose_strategy` phase to ground a recommendation in, and a silent no-op
would hide that fact from the model, the same "say what does exist" style
`write_variant`/`get_reference_analysis` already take with an unknown name.

## Alternatives rejected

**Self-reported completion and persisted/enforced state** — covered above, in
Context, since both were rejected specifically against `#7`'s own constraint
rather than for a general reason.

**One `Phase` enum per `WorkflowKind`.** Considered, since `from_scratch` and
`reference`/`hybrid` do not share every phase. Rejected: `Construct`,
`Render`, `Refine` and `Validate` mean the same thing regardless of kind, and
giving `reference`'s `Compare` a different name for a case that does not have
it would invent a distinction with no behavioural difference — a single flat
enum, with `WorkflowKind::phases()` deciding which subset and order applies,
says the same thing without the duplication.

**A per-region strategy recommendation for `hybrid`** (trace the measurable
parts, name which are not). Rejected for this issue: `analysis::Analysis`
reports whole-image facts (`region_count`, not per-region measurability
verdicts), and building a per-region classifier is real additional scope
`#7`'s acceptance criteria do not ask for — "permits mixing" is satisfied by
`ConstructionStrategy::Hybrid` existing and always being recommended for that
kind, not by deciding which specific regions to trace.

## Verified

`src/workflow/mod.rs`'s own tests cover: `from_scratch`'s phase list excludes
`analyse`/`choose_strategy`/`compare`; `reference` and `hybrid` share one
phase sequence; `compare` always precedes `validate` in that sequence; a
measurable reference (few regions, one colour) is recommended `Trace`; a
complex one (many regions, many colours) is recommended `Construct`; `hybrid`
recommends `Hybrid` for both a clean and a busy reference alike; each enum's
`Display` matches the string the tool's JSON schema and output use.

`src/tools/builtin.rs` covers `get_workflow` itself: phases for `from_scratch`
without a `src`; `reference`'s phases naming `compare` before `validate`; a
strategy recommendation appearing only when `src` is given, computed from a
real traced/analysed fixture image; `hybrid`'s recommendation staying
`Hybrid` regardless of the fixture; refusing `src` for `from_scratch`;
refusing an unrecognised `kind`; refusing an unattached reference by name, the
same shape `get_reference_analysis`'s equivalent test takes. The pinned tool
name list (`src/tools/mod.rs`) and the full-registry assertion
(`tests/mcp_stdio.rs`) both gained `get_workflow` deliberately, not as a side
effect.

## Consequences

- A new top-level module, `src/workflow/`, and a new tool, `get_workflow` —
  seventeen tools total, still alphabetically ordered by
  `Registry`'s `BTreeMap`.
- No project-format change: nothing here is persisted, and opening/saving a
  project is unaffected (invariant 2).
- No enforcement: an agent can still call `write_svg` before `render_svg`
  before anything else, for any workflow kind. That is deliberate — see
  Context — and issue `#8` (system instructions) and `#9` (tool contracts)
  are where the epic's own agent-facing guidance and per-tool sequencing hints
  are addressed, not here.
- `recommend_strategy`'s two thresholds (`MEASURABLE_MAX_REGIONS = 6`,
  `MEASURABLE_MAX_COLOURS = 4`) are a starting judgement call, not a tuned
  result against real references — issue `#10`'s fixtures are where that gets
  proven or corrected.
- Explicit reference reconstruction issue `#7` asks for a transport/UI-
  independent workflow model distinguishing `from_scratch`/`reference`/
  `hybrid`, structural visual-validation-before-completion for reference
  work, tracing never forced on `from_scratch`, mixing permitted for `hybrid`,
  and consumability through the current MCP/ACP path without a UX change —
  all five are what `WorkflowKind::phases()`, `recommend_strategy` and
  `get_workflow` together close.
