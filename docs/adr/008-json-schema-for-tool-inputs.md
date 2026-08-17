# ADR-008 — Tool inputs are described by JSON Schema

## Status

Accepted

## Context

[ADR-004](004-tools-not-a-model.md) gave each tool a hand-rolled parameter
list, and said so deliberately:

> Not a JSON Schema type, deliberately: a transport that needs one can build it
> from this, and the alternative is a schema dependency in a crate whose job is
> drawing logos.

That was the right call at the time, because there was no transport. There is
one now — [ADR-011](011-driving-an-agent-is-still-not-a-model.md) — and MCP's
wire format for a tool's arguments *is* a JSON Schema object. So the sentence
"a transport that needs one can build it from this" has to be cashed in, and it
does not cash.

`Parameter` carried a name, a sentence and a required flag. It had no type. To
synthesise a schema from it, the MCP server would have to assume every argument
is a string, which is wrong for every dimension Shaipe takes, or invent a
convention mapping names to types, which is a second contract living in the
transport where nobody would look for it.

The tools this change adds make the gap concrete. `render_grid` takes an
**array of integers**. `render_svg`'s `background` takes a string with a
**specific grammar** (`transparent`, or a CSS hex colour). `write_svg` must say
that its `source` is the whole document. None of that is expressible in a
triple of name, sentence and boolean.

## Decision

`Tool::parameters() -> &'static [Parameter]` is replaced by
`Tool::input_schema() -> serde_json::Value`, returning a JSON Schema object.
`Parameter` is deleted.

Schemas are **hand-written**, using three small helpers — `object`, `string`,
`integer` — that keep them uniform. Every schema sets
`additionalProperties: false`, so a model that invents an argument is told
rather than silently ignored.

No new dependency: `serde_json` was already here, and a schema is a `Value`.

`Tool` also gains `mutates()`, and `Tool::call` returns a `ToolOutput` — a JSON
value plus a list of images — rather than a bare `Value`. An image is not a
JSON value: base64 inside a JSON string is a string as far as every transport
is concerned, and a model cannot see a string. MCP carries images as their own
content blocks, so the bytes now travel beside the JSON instead of inside it.

`ToolImage` holds **raw PNG bytes, not base64**. The workspace wants to show
the same image it hands an agent, and making it decode base64 to do so would be
encoding for a transport that is not involved.

## Alternatives rejected

**Derive the schema with `schemars`.** The obvious answer, and the one the MCP
Rust SDK's macros assume. Rejected because a tool's schema is *prompt text*, as
much as its description is, and a derived schema carries Rust's vocabulary into
a place a model reads: `Option<u32>` becomes `["integer", "null"]`,
`format: "uint32"` appears, doc comments arrive with their line breaks intact.
Writing the schema by hand costs a dozen lines per tool and keeps the text
addressed to its actual audience. It also keeps `schemars` and the SDK's macro
features out of the build.

**Keep `Parameter` and synthesise a schema in the MCP layer.** This is what
ADR-004 anticipated. Rejected because the synthesis needs type information
`Parameter` does not carry, so the types would have to be re-declared in the
transport — the same tool described in two places, which is precisely what the
registry exists to prevent.

**Extend `Parameter` with a type enum.** A smaller change, and it looked
attractive until `render_grid` needed an array of constrained integers and
`render_svg` needed a documented string grammar. At that point the enum is a
worse JSON Schema with a Shaipe-specific spelling, and something still has to
translate it. Reinventing a standard the consumer already speaks is work spent
moving away from the consumer.

## Consequences

- `Tool` is a breaking change for anyone implementing one outside this crate.
  There is no such implementor yet, and the trait was never published.
- Schemas are prose that can rot. `every_tool_publishes_a_schema_a_model_can_read`
  in `src/tools/mod.rs` asserts that every argument carries a description of
  real length and that nothing is required without being described, so the rot
  is caught rather than shipped.
- `ToolOutput` gives the workspace a way to display what an agent was shown,
  which it did not have when the answer was base64 in a JSON field. That is not
  why the change was made, but it is the reason it will not be reverted.
- ADR-004's parameter-list decision is amended, not overturned. Its actual
  claim — that Shaipe describes operations rather than hosting a model — is
  untouched.
