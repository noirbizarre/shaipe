# ADR-011 — Driving an agent is still not providing a model

## Status

Accepted

## Context

[ADR-004](004-tools-not-a-model.md) is the load-bearing decision in this
project. It says Shaipe provides tools and does not provide a model, and it
rejects, in terms, "a provider integration" and "implementing MCP now".

This change adds an MCP server and an ACP client, and makes `shaipe` start
`opencode acp` as a subprocess. At a glance that is the thing ADR-004 forbids.
It is worth being explicit about why it is not, because the next person to read
both will have the same reaction, and because the day that reasoning stops
holding is the day this project has become something else.

ADR-004's actual argument was about **ownership**, not about wires:

> The user already has an agent — Codex, Claude Code, OpenCode, something
> newer. That agent already has a model, a key, a conversation, a permission
> model and a UI. What it does not have is the ability to *see* an SVG […]
> That gap — rendering and introspection — is the actual product.

Its rejection of MCP was explicitly a *not yet*:

> A transport with no consumer, whose protocol is still moving. It is additive
> later, and cheap, precisely because the registry exists.

Both conditions have changed. The protocol settled: ACP negotiates a stable
wire version 1, and MCP's Rust SDK reached 3.x. And a consumer exists, because
Shaipe itself is one — the workspace is exactly the thing that wants to hand an
agent a set of tools and watch what it does with them.

## Decision

Shaipe speaks two protocols, in two directions, and owns neither end of the
model.

```text
                       Shaipe
                         │
                        ACP          Shaipe is the client.
                         │           The agent is a process the user installed.
                     opencode acp
                         │
                    the user's model, the user's key, the user's bill
                         │
                        MCP          Shaipe is the server.
                         │
                 Shaipe's asset tools
```

What Shaipe does **not** do, and this is the whole of the argument:

- It does not hold an API key, or read one, or ask for one.
- It does not name a provider or a model. It cannot tell you which model
  answered; it never sees one.
- It does not store a conversation. The transcript lives for the session.
- It does not bill anyone. The agent is configured and authenticated by its
  own user, in its own tool.
- It does not require a plugin, an adapter, or anything OpenCode-specific.
  `--agent "<command>"` takes any ACP agent; OpenCode is the first target
  because it is the one this was tested against.

What it does is start a program the user already trusts, and offer it the
ability to see an SVG. That is the gap ADR-004 named.

`shaipe render` still runs with no agent, no MCP, no network and no runtime
dependency of any kind, and CI proves it on every push.

## Alternatives rejected

**A provider integration — `shaipe generate` with an API key.** Still rejected,
for every reason in ADR-004. This change makes it *less* attractive, not more:
the user's agent already has a better conversation, a better permission model
and a better UI than Shaipe would build, and now Shaipe can use them.

**Stopping at the standalone MCP server.** Genuinely tempting — `shaipe mcp` is
most of the value, it is a hundred lines, and it needs no ACP at all. Rejected
because it leaves the interactive loop unproven and leaves Shaipe unable to
dogfood itself: the artwork in this repository is still a hand-drawn
placeholder, and regenerating it *through* Shaipe is the point. It also splits
badly — an agent using the standalone server edits a file the user is not
looking at, which is the divergence [ADR-010](010-mcp-over-a-socket-with-a-bridge.md)
exists to prevent.

**Making OpenCode the architecture — `opencode.rs`, `claude.rs`, `codex.rs`.**
Rejected because the protocol is the abstraction. `src/acp/` knows about ACP
and knows nothing about OpenCode; the only OpenCode-specific things in the tree
are a default command string and a list of session mode names, both of which
degrade to doing nothing on an agent that does not have them.

## Consequences

- The tools are now a public interface in a much stronger sense: a model reads
  their names and descriptions on every turn. [ADR-007](007-tool-names.md)
  and the test that pins the names exist because of this.
- Shaipe must be robust to an agent that is absent, broken, unauthenticated or
  slow, in every case without stopping someone from opening their own project.
  The workspace opens whatever happens, and the reason appears in the prompt
  pane.
- **The workspace cannot wait for the agent's handshake.** An agent starts the
  MCP servers it was given during `session/new` and calls `tools/list` on them
  *before answering it*. Shaipe's MCP server is the live workspace, which can
  only answer from its event loop — so waiting deadlocks the two, the agent
  times out, and the session comes up with none of Shaipe's tools and no error
  anywhere. `Agent::start` therefore returns immediately and reports success or
  failure as an update. `starting_an_agent_does_not_wait_for_it_to_answer` is
  the regression test, and it is not a hypothetical: it is what happened.
- Shaipe asks the agent for a session mode that can actually change things.
  OpenCode opens sessions in `plan`, where the model declines to edit anything
  — a workspace whose entire purpose is making the edit has to ask for
  `build`. This is a preference, not a requirement: an agent offering no such
  option keeps whatever it opened with.
- All model output is untrusted input. An SVG from a model is validated by
  `write_svg` before it replaces anything, and never written to disk without a
  keystroke.
- The end-to-end path cannot be tested in CI without someone's credentials, so
  it is not. `tests/acp_opencode.rs` is `#[ignore]`d and requires an installed,
  authenticated agent; `scripts/smoke-acp.sh` is the manual version.
