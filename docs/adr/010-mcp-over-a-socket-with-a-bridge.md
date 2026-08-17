# ADR-010 — A live workspace serves MCP on a socket, reached by a bridge

## Status

Accepted

## Context

Shaipe exposes its tools over MCP in two situations that look similar and are
not:

**Standalone.** `shaipe mcp logo.svg` opens a project and serves it on its own
stdio. An external client — an editor, another agent — starts it as a
subprocess. This is straightforward, and [ADR-011] covers why it exists.

**Session-scoped.** The user has a project open in the workspace, and asks the
agent for a change. The agent must operate on *that* project: the one on
screen, with the edits the user has already made, so the preview follows what
the agent does and the two never diverge.

The second is the useful one, and it has a hard constraint in the way. ACP
agents start their MCP servers as **subprocesses and speak to them over stdio**
— the only transport the spec requires every agent to support. Verified against
OpenCode 1.18.18: given an `mcpServers` entry in `session/new`, it spawns the
command and completes a full MCP handshake against it.

But the workspace cannot *be* that subprocess. It is already running, it owns
the project, and its own stdin and stdout are the terminal it is drawing on.
Writing JSON-RPC there would corrupt the display; reading it would fight the
key handler.

So the process the agent starts cannot be the process that has the project.

## Decision

The workspace listens on a socket. The agent is given a subprocess that
connects to it and copies bytes.

```text
workspace ──> Listener on unix:/tmp/shaipe-XXXX/mcp.sock
                        ▲
                        │ socket
                        │
agent ──stdio──> shaipe mcp --bridge unix:/tmp/shaipe-XXXX/mcp.sock
```

`session/new` carries
`McpServer::Stdio { command: <current exe>, args: ["mcp", "--bridge", <address>] }`.

Three things make this work:

**The bridge is a pipe, not a server.** `src/mcp/bridge.rs` connects, splits,
and runs `tokio::io::copy` in both directions. It parses nothing and frames
nothing, so a change to MCP's wire format cannot break it and it cannot corrupt
a message it does not understand. It is about forty lines.

**It exits when either direction closes**, via `select!` rather than
`try_join!`. An agent closing its stdin is the normal way an MCP server is told
to exit; waiting for the other direction to notice would leave one orphaned
process per session, which is a leak nobody would think to attribute to this
file.

**The command is `std::env::current_exe()`, not `"shaipe"`.** The workspace the
agent must reach is *this* build. A `shaipe` on `PATH` might be a different
version, or absent entirely during development.

The socket lives in a per-workspace temporary directory, chmodded to `0700`,
with the socket itself at `0600`. Dropping the `Listener` removes both.

`--bridge` is `hide = true` in the CLI. A flag no human should type does not
belong in `--help`.

## Alternatives rejected

**Serve MCP over Streamable HTTP and pass `McpServer::Http { url }`.** The most
serious alternative, and genuinely available: OpenCode advertises
`mcpCapabilities: { http: true, sse: true }`, so this would work today.
Rejected on three counts. It opens a loopback port, which every process on the
machine can reach, to replace forty lines of `copy` on a socket with filesystem
permissions. It adds an HTTP server and its dependencies to a crate that draws
logos. And `http` is a *capability* — an agent may not have it, whereas stdio
is the one transport the spec says every agent MUST support. Choosing the
optional transport over the mandatory one to avoid a small file is a poor
trade. If an agent ever appears that cannot start subprocesses, this is the
answer, and it needs its own ADR.

**MCP over the ACP connection itself (`McpServer::Acp`).** Exactly the right
shape: no socket, no subprocess, the MCP traffic multiplexed over the channel
that already exists. Rejected because it is not available. The variant is
`#[cfg(feature = "unstable_mcp_over_acp")]` in the Rust SDK, described as "not
part of the spec yet, and may be removed or changed at any point", and gated on
an agent advertising `mcpCapabilities.acp`, which no agent surveyed does. Worth
revisiting when it stabilises.

**Have the agent connect to a globally configured `shaipe mcp`.** What a user
would get by putting Shaipe in their `opencode.json`. Rejected because it
defeats the entire purpose: that server opens the file from disk into its own
`Project`. The agent would edit one copy while the user watched another, and
they would diverge silently from the first `write_svg`. This is the failure the
session-scoped mode exists to prevent.

**Make the bridge a second MCP server that proxies at the protocol level.**
Rejected because it would have to understand MCP to forward it, which means a
protocol version to keep in step, a second place for a schema to be wrong, and
a component that can corrupt messages. The pipe cannot.

## Consequences

- One extra process per agent session, doing nothing but `copy`. Cheap, and it
  exits with the agent.
- The two MCP modes share `Server`, `SessionHandle` and every tool, and share
  no lifecycle: one opens a file and owns it, the other reaches a project
  somebody else is looking at.
- Several connections can be open at once — an agent may make more than one —
  and they all reach the same project, because they share a `SessionHandle`.
  Each gets its own task, so one hanging up does not disturb the others.
- On a non-unix platform the fallback is a loopback TCP port, which is weaker.
  It is behind `cfg`, and the comment says so rather than pretending otherwise.
- The socket's permissions are set explicitly and not inherited.
  `tempfile::tempdir` creates through `mkdir`, so its mode is whatever the
  umask leaves — 0755 on a normal system. That was an assumption in the first
  draft of this ADR, and
  `a_workspace_socket_is_not_reachable_by_other_users` is what caught it.

[ADR-011]: 011-driving-an-agent-is-still-not-a-model.md
