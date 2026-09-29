# Architecture Decisions

Records of the decisions that shape this project, and — more usefully — the
reasons behind them. An ADR is written when a choice is hard to reverse or
likely to be re-proposed.

The point is not the decision; it is the alternatives that were rejected and
why. A record that only states the outcome saves nobody the argument.

A decision is changed by writing a new ADR that supersedes the old one, never
by editing the old one's reasoning. The history is the value. Where a decision
still holds but its surroundings grew, an *Update* section is appended instead,
and the original text is left alone. Two small edits in place are tolerated
because they add a pointer rather than change a word of the argument: a
forward link from an older ADR to the one that extends it, and a dated
amendment to an ADR whose subject is a list that keeps growing (ADR-007's
verbs). Anything else is an *Update*.

## Format

`NNN-kebab-case-title.md`, numbered in the order written, with the sections:

- **Status** — Proposed, Accepted, or Superseded by ADR-NNN, optionally
  followed by what it supersedes or which later ADR extended its scope
- **Context** — the forces in play, before any decision
- **Decision** — what was decided
- **Consequences** — what this costs, including what it makes harder

## Index

- [ADR-001](001-svg-as-source-of-truth.md) — The project SVG is the source of truth
- [ADR-002](002-preview-backends.md) — The terminal preview is hand-written, behind a trait *(superseded by ADR-006)*
- [ADR-003](003-variant-isolation.md) — A variant is rendered by isolating it, not by rendering a node
- [ADR-004](004-tools-not-a-model.md) — Shaipe provides tools; it does not provide a model
- [ADR-005](005-declared-fonts.md) — Fonts are declared by the project, not resolved from the system
- [ADR-006](006-ratatui-image.md) — Use `ratatui-image` for terminal previews
- [ADR-007](007-tool-names.md) — Tool names are `verb_noun`
- [ADR-008](008-json-schema-for-tool-inputs.md) — Tool inputs are described by JSON Schema
- [ADR-009](009-session-reached-by-message.md) — The session is reached by message, not by lock
- [ADR-010](010-mcp-over-a-socket-with-a-bridge.md) — A live workspace serves MCP on a socket, reached by a bridge
- [ADR-011](011-driving-an-agent-is-still-not-a-model.md) — Driving an agent is still not providing a model
- [ADR-012](012-cannot-restrict-an-agents-own-tools.md) — Shaipe cannot restrict an agent's own tools, and does not pretend to *(superseded by ADR-013)*
- [ADR-013](013-restrict-the-agent-through-its-environment.md) — Restrict the agent through the environment it is started in
- [ADR-014](014-deterministic-raster-to-vector-tracing.md) — Trace a reference deterministically; do not ask a model to redraw it
  *(its final section is the overview of the reconstruction workflow)*
- [ADR-015](015-checksum-pinned-remote-fonts.md) — A font may be declared by a checksum-pinned URL, cached locally
- [ADR-016](016-remembering-a-chosen-model.md) — A chosen model is remembered per agent, in the OS config directory
- [ADR-017](017-vision-capability-from-models-dev.md) — Vision capability is read from models.dev, cached, never guessed
- [ADR-018](018-structured-reference-measurement.md) — Measure a reference's pixels into facts; do not ask a model to guess them
- [ADR-019](019-compare-reference.md) — Compare a reference and a render on a canvas the tool chooses, never the model
- [ADR-020](020-richer-reference-tracing.md) — Richer tracing: colour regions, and measuring the SVG a trace produces rather than the pixels behind it
- [ADR-021](021-explicit-reconstruction-workflow.md) — Name the reconstruction workflow's phases; do not track whether they happened
- [ADR-022](022-reconstruction-instructions.md) — Tell the agent how to reconstruct in Shaipe's own instructions, generated from the workflow model
- [ADR-023](023-tool-contract-conventions.md) — Tool contracts name what comes next and leave the phases to one place
- [ADR-024](024-reconstruction-fixtures-and-evaluation-loop.md) — Reconstruction is tested with computed fixtures, no model
