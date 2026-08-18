# ADR-012 — Shaipe cannot restrict an agent's own tools, and does not pretend to

## Status

Superseded by [ADR-013](013-restrict-the-agent-through-its-environment.md).

Its analysis of the protocol holds: an ACP client has no lever, and permission
is resolved inside the agent. Its conclusion does not. Shaipe spawns the agent,
and a parent chooses the environment its child starts in — a different layer,
with a different answer. Kept for the analysis, and because the shape of the
mistake (reasoning about one layer and concluding about the system) is worth
remembering.

## Context

The intent was to make Shaipe's tools the only way an agent can change the
project: expose `write_svg`, and refuse the agent's own editor and shell. The
workspace is the thing that knows how to validate an SVG, keep the preview in
step and leave the working tree alone until asked, and an agent going around
all of that is an agent that can quietly undo every guarantee Shaipe makes.

ACP appears to offer the lever. Shaipe is the client, and the client is who the
agent asks: `session/request_permission` arrives with a `ToolCallUpdate` whose
`kind` is one of `Read`, `Edit`, `Delete`, `Move`, `Search`, `Execute`,
`Think`, `Fetch`, `SwitchMode`, `Other`. Refusing the four that modify things
looks like exactly the right answer.

It is not available. Probed against OpenCode 1.18.18:

- Asked to call `get_variants`, **with no `--yes`**, it did so. No permission
  request arrived.
- Asked to write a file with its own tool, **still with no `--yes`**, it wrote
  it. No permission request arrived. The file was there afterwards.

Zero `session/request_permission` calls in either case. OpenCode's own
documentation says why:

> Most permissions default to `"allow"`.

Permission is resolved **inside the agent**, against the agent's configuration,
before the client is involved. A client is asked only about what that
configuration marks `"ask"`. A client that is never asked cannot refuse.

This also means `--yes` was close to a no-op with a default OpenCode install,
while its help text implied it was the thing standing between an agent and the
user's files. That was the most misleading sentence in the project.

## Decision

**Shaipe does not claim to restrict what an agent can do.** It does three
things instead, in descending order of how much they are worth.

**It notices.** `src/tui/watch.rs` polls the project's `mtime` and length on
the tick that already runs. An agent that writes the file with its own editor
is picked up and the preview follows; if there is unsaved work the workspace
refuses to choose, says so, and declines one save rather than overwriting
somebody's work with bytes read before they wrote. This is the part that
actually protects the user, and it protects them from a text editor in another
window just as well as from an agent.

**It asks nicely.** The MCP preamble tells the agent to change the project
through `write_svg` and not with a file editor, and says why: the workspace is
holding the same document, and a direct write goes around its validation.
Prompt text is not a guarantee, and is not described as one.

**It still answers correctly when it is asked.** `Policy` is kept and made
honest — `Guarded` refuses the modifying kinds and always permits Shaipe's own
tools, `AllowAll` is `--yes`, `DenyAll` refuses everything. This is dead code
against a default OpenCode install and live code against an agent, or a
configuration, that does ask. Refusals select a `RejectOnce` option rather than
answering `Cancelled`, because an agent reads the latter as *the user went
away* and the former as *no, try something else*.

Real enforcement belongs to the layer that owns it, which is the agent's own
configuration. For OpenCode that is:

```jsonc
// opencode.json
{
  "permission": { "edit": "deny", "bash": "deny" }
}
```

This is documented in the README rather than implemented, because it is the
user's file and their decision.

## Alternatives rejected

**Write an `opencode.json` for the user.** Shaipe knows the agent's working
directory and could drop a config beside the project with `edit: deny`.
Rejected twice over: it writes to the user's tree, which is the exact thing the
rest of this design refuses to do, and it would silently override their model,
provider and permission settings. A tool that fixes a safety problem by editing
your configuration behind your back has not made you safer.

**Point the agent at a generated config through the environment.** Cleaner than
writing to the tree, and still wrong: it replaces the user's configuration
rather than merging with it, and it is OpenCode-specific in a module whose
whole claim is that it speaks a protocol rather than to a product.

**Select a restricted agent through ACP's `mode` option.** OpenCode's modes are
its agents, and one could be defined with no edit tools. But Shaipe can only
choose from the values it is offered; it cannot define one. That makes it a
recipe for the user, not a mechanism — and it is already in the README beside
the permission block.

**Say nothing and let `--yes` keep implying it was a guard.** Rejected. The
flag's help now says what it does and does not do.

## Consequences

- An agent can still write to the working tree. That is a property of the
  agent, is now stated plainly in the README and in `--yes`'s help, and is no
  longer something the user could reasonably read Shaipe as preventing.
- File watching moves from a nicety to the thing this ADR leans on, which is
  why it landed first.
- `Policy::Guarded` is untestable end to end against a default OpenCode, so it
  is covered by unit tests over `decide` and marked in `PLAN.md` as unproven
  against a real agent that asks. A green test that proves nothing is worse
  than no test, and this one is honest about which half it proves.
- If a future agent or configuration does ask, Shaipe answers correctly on the
  first turn without a change.
