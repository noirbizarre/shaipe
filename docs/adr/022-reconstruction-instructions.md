# ADR-022 — Tell the agent how to reconstruct in Shaipe's own instructions, generated from the workflow model

## Status

Accepted

## Context

ADR-021 named the phases of reconstruction and `get_workflow` reports them, but
only to an agent that already thinks to ask. Nothing told an agent to ask, to
inspect a reference before editing, to measure rather than estimate, or that a
`write_svg` that parses is not a result that matches. Issue `#8` is that layer.

There are three different things an agent reads, and they must stay apart:

- the project's **prompt** — the user's design brief, in the document's
  metadata, edited in the prompt pane and committed with the project;
- a **turn** — what is sent to the agent this time. Today that is the prompt,
  wrapped by `instruct()` in `src/tui/app.rs`, sent with `a`; there is no
  separate chat message any more;
- **Shaipe's own instructions** — how the tools are meant to be used. Neither
  the user's nor the project's, so neither of the above is where they go.

ACP has no system-prompt field: `session/new` carries a working directory and
MCP servers, and `_meta` is reserved for extensions an agent may ignore. The
one channel every agent shares is the MCP server's own `instructions`, returned
from `initialize`, which Shaipe already used for a short preamble about
project mechanics.

## Decision

**The reconstruction instructions are text in `src/workflow/instructions.rs`,
sent as the MCP server's `instructions`, after the existing preamble.**

- They live in `workflow`, not `mcp`: `mcp` is a transport, and text there is
  text the CLI and workspace cannot reach. `Server::get_info` only concatenates.
- They are built, not a constant, so that the phase sequences are spliced in
  from `WorkflowKind::phases()`. The instructions cannot disagree with
  `get_workflow` about the order, and a test checks that they and its
  description state the same one.
- They are the same on every call and for every project: no project state, no
  agent identity. They name no agent product, which a test enforces.
- They cover: choosing `from_scratch`, `reference` or `hybrid`; inspecting a
  reference before editing; measuring with `get_reference_analysis` and
  `get_reference_trace` rather than estimating geometry from pixels; when to
  trace and when to construct; the write, render, compare loop; that a valid
  SVG is not a finished one and that `reference`/`hybrid` work ends only after
  `compare_reference`; and keeping the result editable rather than embedding
  or copying raster noise.
- A test checks that every tool the text names exists, and that the tools the
  loop needs are all named.

**`instruct()` carries no workflow.** It stays the short per-turn wrapper and
is changed in one place: when references are attached it now points at
`get_references` and, for a `source`, at `get_workflow`, instead of suggesting
the agent look at whichever files seem relevant. This is also the fallback for
an agent that does not surface MCP `instructions` to its model.

## Alternatives rejected

**Appending the workflow to the user's prompt or to every turn.** It is the
mixing the issue forbids: the prompt is committed with the project, and text in
every turn is a second copy that drifts from the tested one.

**Injecting it through OpenCode's configuration.** ADR-013 does that for
permissions, where nothing else can work. Instructions do have a generic
channel, and a product-specific one would leave every other agent without them.

**A `_meta` payload on `session/new`.** The specification lets an agent ignore
it.

**Instructions computed from the project.** Whether a reference is attached is
already answered by `get_references`, and the agent chooses the workflow
(ADR-021). Per-project text would make the instructions untestable as one
piece of prose and would change what an agent is told when a file is attached.

## Consequences

- Both serving modes send it, because both use `Server::get_info`.
- **Unverified against a real agent.** MCP `instructions` reaching a model is
  agent-dependent, and this ADR has not probed any agent for it. The
  `instruct()` pointer to `get_workflow` is what covers an agent that drops
  them. Anything asserted about a model following them belongs in an ignored
  test like `tests/acp_opencode.rs`, not in `mise run ci`.
- Renaming a tool now means editing the instructions too; the test says so.
- The instructions are guidance, as ADR-021's phases are. Nothing enforces them.
- Tool descriptions were unchanged by this ADR. Reviewing them for consistency
  with the instructions, and for sequencing hints, is [ADR-023](023-tool-contract-conventions.md).
