# ADR-004 — Shaipe provides tools; it does not provide a model

## Status

Accepted

## Context

Shaipe is described as LLM-native, which invites an obvious reading: that it
should talk to a model. Pick a provider, add an API key, implement `shaipe
generate`.

That reading is wrong, and building it would be the most expensive mistake
available here. The user already has an agent — Codex, Claude Code, OpenCode,
something newer. That agent already has a model, a key, a conversation, a
permission model and a UI. Shaipe reimplementing any of that produces a worse
version of a solved problem, and creates an account and a bill nobody asked
for.

What the agent *cannot* do is see an SVG. It can read the markup, but it cannot
tell whether the result looks like a logo, whether the wordmark is legible at
512 pixels, or whether the mark still reads at 16. It also has no way to learn
what a project's variants and colours are called.

That gap — rendering and introspection — is the actual product.

## Decision

Shaipe provides tools. It does not provide a model, a provider, a key or a
conversation.

`src/tools/` defines a `Tool` trait and a `Registry`: a name, a description
written as prompt text, a parameter list, and a call taking JSON and returning
JSON.

**There is deliberately no transport.** An MCP server, a JSON-RPC endpoint or a
subprocess protocol would each *call* `Registry::call`; none of them is
something this crate has to be. Building one now would mean inventing
requirements for a consumer that does not exist yet.

Everything currently registered is read-only: `inspect_project`,
`list_variants`, `inspect_palette`, `render`. `Tool::call` takes the project by
`&mut` anyway, so the mutating tools that come next need no new trait.

> **Since written:** those four tools were renamed by
> [ADR-007](007-tool-names.md), their parameter lists became JSON Schema in
> [ADR-008](008-json-schema-for-tool-inputs.md), and the transport this section
> declines to build now exists — see
> [ADR-011](011-driving-an-agent-is-still-not-a-model.md), which argues that
> building it does not contradict this ADR. The decision recorded here is
> unchanged: Shaipe still provides tools and still does not provide a model.

### Alternatives rejected

- **A provider integration.** Rejected above.
- **No abstraction at all — let agents shell out to `shaipe` and parse
  stdout.** Tempting, and it works today. But it makes every tool's contract
  the accident of a print statement, and it means the CLI's output format can
  never change. Naming the operations in one place is what lets the CLI, a
  future MCP server and the workspace expose the same semantics.
- **Implementing MCP now.** A transport with no consumer, whose protocol is
  still moving. It is additive later, and cheap, precisely because the registry
  exists.

## Consequences

- There is no `shaipe generate`, and the README says so rather than implying
  otherwise. A capability that is described but absent is worse than one that
  is simply missing.
- Tool descriptions are prompt text and are load-bearing: a tool whose
  description is vague is a tool the model will call at the wrong time. A test
  asserts every registered tool has one.
- Tool names are a public interface in the same way diagnostic codes are.
  Renaming one is a breaking change.
- The registry's order is stable, because that list becomes part of a prompt
  and a set that reorders itself makes a model's behaviour irreproducible for
  no reason.
- Rendering stays completely independent of all of this. `shaipe render` in CI
  never constructs a registry, and nothing in `src/render/` knows the module
  exists.
