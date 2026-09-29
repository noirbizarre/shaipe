# ADR-023 — Tool contracts name what comes next and leave the phases to one place

## Status

Accepted

## Context

The reference, render and write tools were added one at a time, each by an issue
with its own brief, and each description was written to be right about its own
tool. Read together they were not a coherent API:

- `get_project` said it was "the only way" to learn the variant names, and
  `get_variants` lists them. `render_svg` said it was "the only way" to see the
  artwork, and `render_grid`, `compare_reference` and a trace preview all show
  it. A model that believes either skips the tool that would have helped.
- `get_reference_trace` said to use it "before hand-writing a mark from a
  `source` reference", while the instructions ([ADR-022](022-reconstruction-instructions.md))
  say to trace only a clean, flat-colour mark. The description won, because the
  description is what a model reads on every listing.
- Nothing said the traced SVG has no `viewBox`, that `compare_reference` draws
  on a transparent background, or what its overlay colours mean.
- `write_svg` and `write_variant` returned `saved: false` and nothing else. The
  step after a write is a look, and the result was silent on it.
- A rejected write said the SVG "is not a valid Shaipe project" and dropped the
  reason: the cause is the error's `#[source]`, which the MCP adapter never sent.
  The one help text also told `write_variant` to resend the whole document.
- `write_svg` was described, and commented, as checking the document is
  renderable. It constructs a `Renderer`, which parses the XML and loads fonts
  and resolves no variant.
- Its note said "Nothing has been written to disk", which is false under
  `shaipe mcp --write`.
- Optional numbers the schema bounds (`threshold` up to 255, `max_colors` of at
  least 1) were silently ignored when out of range.

## Decision

**A contract says what the tool does, what it needs first, and which tool to call
next.** Four rules, each with a test:

1. **Contracts point at each other; they do not repeat the workflow.** Every tool
   on the reconstruction loop names its neighbours by their real names. The order
   of the phases has one prose home, `get_workflow`, whose description is already
   checked against the instructions; no other contract may state it
   (`no_contract_but_the_workflows_restates_the_phase_order`). The loop is
   asserted as a table of links
   (`the_reconstruction_loop_is_followable_from_the_contracts_alone`), and every
   tool a description or argument names must exist
   (`every_tool_a_contract_names_exists`).
2. **No overclaims.** "The only way" and "call this first" are rejected outright
   (`no_contract_claims_to_be_the_only_way_or_the_only_first_step`). A tool is
   described by what it does and what it is not for.
3. **An argument whose value the model cannot invent says where it comes from.**
   `src` names `get_references` and `variant` names `get_variants`
   (`a_src_or_variant_argument_says_which_tool_lists_its_values`).
4. **Descriptions are bounded.** They are in context on every turn for every tool.
   Guidance about a *parameter* lives in the parameter's own description, not in
   the tool's (`a_description_stays_short_enough_to_be_read_on_every_listing`).

**Results carry the next step.** `write_svg` and `write_variant` return a `next`
that says valid is not finished and names `render_svg` and `compare_reference`.
`render_svg` echoes the background it drew on. `set_palette_colour` returns
`restyled`, the number of bound attributes it rewrote, so zero is distinguishable
from success. `get_reference_image` returns the reference's pixel dimensions, or
null when the header does not decode, so a model knows the size
`compare_reference` will use.

**Errors are complete over the wire.** `explain()` in `src/mcp/server.rs` appends
the error's source chain, and a cause that is itself a Shaipe error brings its
own help. `InvalidSvgFromTool`'s help depends on the tool. The unattached-`src`
refusal is one function for all five reference tools and names both
`get_references` and `set_reference`. A refused colour keeps the parser's
statement of the accepted formats. An unrecognised reference extension is
refused before the file is read. `threshold` and `max_colors` outside their
schema bounds are refused, naming the bound. Diagnostic codes are unchanged.

**The notes say what the call did.** A write's `saved: false` and its note now
say this call did not write, and that the file is touched only under `--write`
or when the user saves.

## Alternatives rejected

**Restate the workflow in each tool.** Six copies of "inspect, analyse, ..." that
can drift from `get_workflow`'s. The links are enough: a model that follows them
walks the loop.

**One shared boilerplate paragraph on every tool.** It would be in context
seventeen times and would say the same thing about tools it does not apply to.

**Derive schemas and descriptions.** Rejected by [ADR-008](008-json-schema-for-tool-inputs.md)
already, and for the same reason: this is prompt text and should be reviewed as
prose.

**Make `saved` reflect `--write`.** Correct, but the tool would need to know how
its session saves, which is the session's business
([ADR-009](009-session-reached-by-message.md)). The wording is honest in the
meantime; the field is a follow-up.

**Make `write_svg` render every variant.** It would catch a vanished variant
element at write time, at the cost of a render per variant per write. The
description says the check parses and does not render, and the result points at
`render_svg`.

## Consequences

- A description that grows past the bound, names a tool that does not exist,
  repeats the phase order, or claims to be the only way fails a test.
- Renaming a tool now needs its neighbours' descriptions edited too, and the
  tests say so.
- Responses gained fields (`next`, `background`, `restyled`, `dimensions`) and a
  changed `note`. None was removed. An error is longer over MCP, and its
  `Caused by:` block is part of what a client sees.
- The contracts are unverified against a real agent, as the instructions are.
  These tests establish that they are consistent with each other and with the
  code, not that a model reads them and behaves.
- Not done: output schemas, MCP `title` and destructive/idempotent hints, and
  making `saved` true under `--write`.
