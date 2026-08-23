# ADR-007 — Tool names are `verb_noun`

## Status

Accepted

## Context

[ADR-004](004-tools-not-a-model.md) states that a tool name is a public
interface:

> Tool names are a public interface. Renaming one is a breaking change, in the
> same way renaming a diagnostic code is.

The first four tools were named as they were written, one at a time:
`inspect_project`, `list_variants`, `inspect_palette`, `render`. Three verbs
for two kinds of operation, and all three mean "read something".

That was invisible while the registry was a library API with no consumer. It
stops being invisible the moment the tools appear in a model's context, because
of how they get there. An agent connected to three MCP servers sees one flat
list. `render` alone, sitting beside a browser's `screenshot` and a shell's
`run`, does not say what it renders or that it is even about images. And a
model deciding whether a tool is safe to call speculatively reads the name
before it reads the description — often it is choosing between a dozen names on
the strength of the names alone.

The set is also about to grow a tool that *writes*. `write_svg` next to
`inspect_project` and `list_variants` gives no clue from the shape of the names
which of them can change the project.

If this is going to be fixed, it has to be fixed before there is a consumer.
Once someone has `shaipe mcp` in a config file and an agent has tool calls in
its history, the cost is real.

## Decision

Every tool is named `verb_noun`, with the verb drawn from a closed set:

| Verb | Meaning |
|---|---|
| `get_` | Reads something and returns it. Never changes the project. |
| `render_` | Rasterises. Returns images to look at. Never changes the project. |
| `write_` | Replaces raw document markup. Changes the project in memory. |
| `set_` | Sets a metadata field to an explicit value. Never touches document markup. Changes the project in memory. |

The renames:

| Was | Is |
|---|---|
| `inspect_project` | `get_project` |
| `list_variants` | `get_variants` |
| `inspect_palette` | `get_palette` |
| `render` | `render_svg` |

`get_svg`, `write_svg` and `render_grid` are added under the same rule.

**Amendment.** `set_` was anticipated but not used when this ADR was accepted
— the closing paragraph named `set_palette_colour` as the example of a tool
that would need it. It now exists, alongside `set_generation`, and this ADR is
edited in place to record it rather than superseded, for the same reason the
original renames were made directly: the crate remains unpublished, so there
is still no config file and no conversation history a verb change could break.
`write_variant` needed no amendment; it replaces one element's markup, which
is exactly what `write_` already meant — `write_svg` replaces all of it,
`write_variant` replaces one element by `id`, spliced the same way
[`replace_metadata`](../../src/project/document.rs) already splices
`<shaipe:project>`.

The names are pinned by `the_tool_names_are_the_ones_the_adr_records` in
`src/tools/mod.rs`, which asserts the exact list. Changing a name means editing
that test, which is the point: it makes the rename a deliberate act rather than
a side effect of a refactor.

## Alternatives rejected

**Keep the existing names and apply the convention only to new tools.** The
literal reading of ADR-004, and the cheapest option. Rejected because the cost
it avoids is hypothetical — the crate is unpublished, `shaipe mcp` does not
exist yet, so there is no config file and no conversation history to break —
while the cost it accepts is permanent. A vocabulary that is half one
convention and half another is one every future reader has to learn twice.

**Register the old names as aliases.** Rejected, and this is the one worth
being explicit about, because it sounds free. It is not: an alias is not a
redirect, it is a *second entry in the tool list*. The list is prompt text sent
on every request. Aliasing four tools means a model reads eight, has to work
out that two of them are the same thing, and pays for the confusion in tokens
on every turn. The compatibility being preserved is for consumers who do not
exist; the cost is paid by every consumer who will.

**Drop the noun where it is obvious — `render` rather than `render_svg`.**
Rejected because it is only obvious inside Shaipe. In the flat list an agent
actually sees, the noun is doing the work of saying whose `render` this is.

## Consequences

- A breaking change to a public interface, deliberately taken while nothing
  consumes it. The changelog carries it as its own entry.
- `render_svg` and `render_grid` share a verb, which correctly says they do the
  same kind of thing at different scales.
- The verb set is closed, so adding a tool means either fitting it or extending
  this ADR — `set_palette_colour` and `set_generation` are the fourth verb,
  `set_`, added by amendment above; that is the intended way to grow it.
- `get_` promises no mutation, and `Tool::mutates()` says the same thing in
  code. They can disagree.
  `only_the_writing_and_setting_tools_declare_that_they_mutate` in
  `src/tools/builtin.rs` is what stops them.
