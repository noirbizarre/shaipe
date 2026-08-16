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
- [ADR-002](002-preview-backends.md) — The terminal preview is hand-written, behind a trait
- [ADR-003](003-variant-isolation.md) — A variant is rendered by isolating it, not by rendering a node
- [ADR-004](004-tools-not-a-model.md) — Shaipe provides tools; it does not provide a model
- [ADR-005](005-declared-fonts.md) — Fonts are declared by the project, not resolved from the system
