# Architecture Decisions

Records of the decisions that shape this project, and — more usefully — the
reasons behind them. An ADR is written when a choice is hard to reverse or
likely to be re-proposed.

The point is not the decision; it is the alternatives that were rejected and
why. A record that only states the outcome saves nobody the argument.

A decision is changed by writing a new ADR that supersedes the old one, never
by editing the old one. The history is the value.

## Format

`NNN-kebab-case-title.md`, numbered in the order written, with the sections:

- **Status** — Proposed, Accepted, or Superseded by ADR-NNN
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
- [ADR-012](012-cannot-restrict-an-agents-own-tools.md) — Shaipe cannot restrict an agent's own tools *(superseded by ADR-013)*
- [ADR-013](013-restrict-the-agent-through-its-environment.md) — Restrict the agent through the environment it is started in
- [ADR-014](014-deterministic-raster-to-vector-tracing.md) — Trace a reference deterministically; do not ask a model to redraw it
- [ADR-015](015-checksum-pinned-remote-fonts.md) — A font may be declared by a checksum-pinned URL, cached locally
