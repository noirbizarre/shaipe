# ADR-030 — Keep durable description separate from agent messages

## Status

Accepted; extends ADR-001 and ADR-022

## Context

The original `<shaipe:prompt>` was both a saved design brief and the message
sent with `a`. A new agent session could read the brief, but using it as the
conversation also meant that a one-off request changed the project's source of
truth. The display transcript is session state and must not end up in the SVG.

## Decision

Schema version 2 names durable visual intent `<shaipe:description>`. The
metadata model exposes it as `description`, independent of XML callers. Version
1 `<shaipe:prompt>` is still read as the description. An untouched legacy
document remains byte-identical, including on save; editing its metadata
serializes version 2. Empty or whitespace-only descriptions are absent;
internal line breaks survive.

`get_project` reports the description and `set_description` replaces it without
touching the artwork. The TUI gives description and message their own editors.
The latter and the display transcript live only for the session. Sending a
message supplies the current description as context on every user-initiated
turn, without replaying earlier messages or waiting for the ACP handshake.
Shaipe-owned workflow instructions continue to travel through MCP instructions.
`shaipe init --description` seeds the metadata; `--prompt` is removed because
`init` has no agent session in which to send a transient turn.

## Consequences

Opening a legacy project does not churn the working tree, but its first
metadata edit upgrades its schema. An agent may update intent incrementally by
reading the current description, then sending its replacement through
`set_description`; it need not rewrite SVG geometry or serialize a transcript.
Messages are lost when the workspace closes, deliberately. The first user turn
is required to ask the agent to act; simply opening a project starts no turn.
