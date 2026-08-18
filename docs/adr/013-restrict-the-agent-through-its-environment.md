# ADR-013 — Restrict the agent through the environment it is started in

## Status

Accepted. Supersedes [ADR-012](012-cannot-restrict-an-agents-own-tools.md).

## Context

ADR-012 concluded that Shaipe cannot restrict an agent's own tools, and settled
for noticing what one did afterwards. The evidence behind it was sound and the
conclusion was too broad.

What it established is true: **the protocol** offers no lever. `initialize`,
`ClientCapabilities` and `session/new` contain nothing that restricts an agent,
every capability field is a positive declaration of what the *client* supports,
and permission is resolved inside the agent, against the agent's configuration,
before a client is consulted at all. OpenCode's permissions default to `allow`,
so it never asks, and a client that is never asked cannot refuse.

What it missed is that Shaipe is not only a client. **It spawns the process**,
and a parent chooses the environment its child starts in. That is a different
layer, and the answer there is different.

Confirmed against OpenCode 1.18.18:

- `AcpAgentConfig::env`/`envs` set variables on the spawned child, on top of an
  inherited environment.
- OpenCode reads `OPENCODE_CONFIG_CONTENT` and **deep-merges** it into its own
  configuration. Verified against a real user config: `permission.edit` was set
  while the model, provider, MCP servers, plugins and every existing
  `permission.read` rule survived untouched.
- It is layer 6 of 8 in that resolution — above a project's own
  `opencode.json`, below enterprise managed configuration.

And end to end, with `{"permission": {"edit": "deny", "bash": "deny"}}`:

- Asked to create a file with its own tools, the agent answered *"I don't have
  a bash/shell tool or file-writing tool available in this session"*, and no
  file appeared. OpenCode does not refuse a denied tool — it **removes** it, so
  the model never proposes it.
- Asked to change a colour through `shaipe_get_svg` and `shaipe_write_svg`,
  both completed. Denying `edit` does not touch MCP tools, because OpenCode's
  permission keys are its own built-in tool names.

## Decision

Shaipe starts OpenCode with `edit` and `bash` denied, **always**.

```jsonc
// merged into OPENCODE_CONFIG_CONTENT
{ "permission": { "edit": "deny", "bash": "deny" } }
```

`edit` covers edit, write and patch. `bash` closes the hole the first one
leaves: an agent that finds `edit` denied will reach for `echo > file`, and a
restriction with a shell-shaped gap in it is decoration. Reading, globbing,
grepping and fetching stay allowed — an agent that can read `AGENTS.md` and the
SVG it is editing does markedly better work, and none of it can damage the
project.

**Not a flag.** A restriction that can be waved away is one that has to be
reasoned about at every call site, and the thing it protects — that `write_svg`
is the only way the project changes, so the document is validated and the
preview stays in step — is not something a user benefits from switching off.
`--yes` does not disable it and could not: OpenCode enforces explicit `deny`
regardless of auto-approval.

Three things make it honest rather than merely effective:

- **It merges.** Whatever `OPENCODE_CONFIG_CONTENT` already held is added to,
  not replaced. Overwriting it would silently discard a configuration somebody
  wrote. A value that does not parse is reported rather than dropped.
- **It says so.** The first thing in the transcript is what the agent may and
  may not do. Reaching into somebody's agent configuration without telling them
  would be worse than not reaching.
- **It knows one agent.** `src/acp/opencode.rs` is the only product-specific
  module in the crate. Anything that is not OpenCode gets no environment and is
  reported as unrestricted, rather than quietly assumed to be safe.

## Alternatives rejected

**`OPENCODE_PERMISSION`.** More surgical — it merges only into `permission` —
and applied later still, after even managed configuration. Rejected because it
is undocumented: present in 1.18.18 with no public-API guarantee. A restriction
that quietly stops working in a later release is worse than one that never
existed, because the user believes it is there.

**Writing an `opencode.json` beside the project.** Shaipe knows the agent's
working directory and could drop a config in it. Rejected twice over: it writes
to the user's tree, which is the exact thing the rest of this design refuses to
do, and a tool that fixes a safety problem by editing your configuration behind
your back has not made you safer.

**Injecting a scoped agent** via `{"agent": {"shaipe": …}, "default_agent":
"shaipe"}`. Verified to work — the session starts in it and the user's own
agents stay selectable. Rejected as more than was asked: it changes which agent
the user is talking to, and with it the model, the prompt and the personality,
to achieve something two permission keys already achieve.

**Leaving it at ADR-012's answer.** Noticing is still worth having and is still
there — it is what covers an agent Shaipe does not know how to restrict, and a
text editor in another window. But noticing after the fact is a weaker promise
than not letting it happen, when both are available.

## Consequences

- The agent cannot edit files or run commands for the whole session. That is
  the point, and it is a real loss: no `git`, no test run, no `mise run assets`.
  For a workspace whose job is one SVG, changed through `write_svg`, that is
  the right trade. If it stops being the right trade, this ADR is where to
  argue with it.
- Shaipe now reaches into another program's configuration. That is a genuine
  coupling, confined to one module and one documented variable, and announced
  to the user on the first turn.
- `--yes` is left meaning only what it always meant underneath: answering
  permission requests from an agent that sends them. With OpenCode that is
  still close to nothing.
- **A residual hole:** OpenCode's `task` tool launches subagents, and agent
  permissions merge with the global config with the agent's rules taking
  precedence — so a user-defined subagent with `edit: allow` could in principle
  get through. The end-to-end test asserts the outcome (no file appears) rather
  than the mechanism, and it holds today. If it ever stops holding, `task` is
  the next key to deny.
- ADR-012 is superseded rather than deleted. Its analysis of the protocol is
  correct and worth keeping; only its conclusion was too wide, and the shape of
  that mistake — reasoning about one layer and concluding about the system — is
  the useful part of the record.
