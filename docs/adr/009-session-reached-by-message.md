# ADR-009 — The session is reached by message, not by lock

## Status

Accepted

## Context

Two things now need to call tools against the same project:

- an MCP server, serving one or more connections from an agent;
- the workspace, which owns the `Project` by value and holds `&mut self`
  across a whole frame while drawing it.

The obvious answer is to put the project behind a lock and hand a clone of the
handle to each. It is the answer almost every codebase reaches for, and it is
wrong here for two separate reasons.

**The mechanical one.** `prek.toml` carries a hook,
`stdout-lock-not-held-across-the-tui`:

```
[ "$(grep -rl "[.]lock()" src --include=*.rs)" = src/main.rs ]
```

It exists because holding stdout's lock across the workspace silently
downgrades every terminal preview to half-blocks — a bug that shipped once and
was expensive to find, because nothing fails, the pictures just get worse. The
hook is textual: it asserts `.lock()` appears in exactly one file. Any mutex,
of any flavour, breaks it.

**The real one.** A lock would be wrong even if the hook did not exist. The
workspace mutates `App.project` between frames and reads it throughout one. A
tool call arriving mid-frame either blocks the draw — a workspace that freezes
whenever an agent is working — or forces `App` apart so the project can live
somewhere the drawing code does not borrow. Both are large changes in service
of a synchronisation primitive that is not needed.

## Decision

The project has exactly one owner, and is reached only by message.

```rust
pub enum SessionCommand {
    List { reply: oneshot::Sender<Vec<ToolDescriptor>> },
    Call { name: String, input: Value, reply: oneshot::Sender<Result<ToolOutput>> },
}

pub struct SessionHandle { commands: mpsc::Sender<SessionCommand> }

pub fn serve(command: SessionCommand, registry: &Registry, project: &mut Project) -> Applied;
```

`SessionHandle` is cheap to clone; one goes to each MCP connection. The owner
services the receiver:

- the **workspace** does it in its `select!` loop, between frames, which is the
  only moment mutating the project is safe;
- **standalone `shaipe mcp`** has no workspace, so `SessionHandle::detached`
  spawns a task that does nothing else.

Both call the same `serve`, so the two cannot drift into different meanings of
the same tool call.

`serve` returns `Applied { mutated }` rather than acting on it. This module
knows nothing about previews or dirty markers and must not learn: the workspace
reads `mutated` and decides for itself that its preview is stale.

The channel is bounded at 16. An agent that outruns a human's workspace should
feel backpressure, not build a backlog of edits against a project it can no
longer see.

## Alternatives rejected

**`std::sync::Mutex<Project>`.** Breaks the hook, and blocks a runtime worker
on a rasterise.

**`tokio::sync::Mutex<Project>`.** Breaks the hook too — `.lock().await` is
still `.lock()` to a grep.

**`tokio::sync::RwLock`, spelled `.write().await`.** This passes the grep. It
is the worst option on the list, and it is recorded here because it is the one
someone will suggest. Choosing an API because its method name evades a safety
check is how the check stops meaning anything: the next person to add a real
`.lock()` for an unrelated reason gets a failure they cannot explain, and the
guard has been quietly converted into a naming convention. It is also still
wrong on the merits, for the frame-borrow reason above.

**Amend the hook to exclude `src/mcp/`.** Honest, at least. Rejected because
the alternative needs no lock at all, so the exemption would be buying nothing.

**Give the MCP server its own `Project`, loaded from the same path.** Simple,
and it makes the tools trivially `Send`. Rejected because it defeats the entire
point of the session-scoped mode: the agent would be editing a second copy of
the file while the user watched the first, and the two would diverge silently
from the first `write_svg`.

## Consequences

- Tool calls are serialised. This is not a cost: `Tool::call` takes
  `&mut Project` and would require it anyway, and it is what makes an agent's
  write-then-read pair do what it says.
- An MCP call arriving while the workspace is blocked writing a Kitty image
  waits for the next frame. That is correct, and it shows up as latency rather
  than as corruption.
- A tool that costs more than a frame would stall the workspace. Nothing does
  today — constructing a `Renderer` is tens of milliseconds — but if one ever
  does, it belongs on `spawn_blocking`, and the measurement should come before
  the assumption.
- Every failure mode of a dead peer becomes `Error::SessionClosed` rather than
  a hang, in both directions: the owner having gone before the command was
  sent, and having gone after accepting it. Both are tested, because an agent
  waiting forever on a tool call is the worst available outcome.
- `tokio` enters the library. It was going to arrive with the MCP server
  regardless; it arrives here first, and for a reason unrelated to transports.
