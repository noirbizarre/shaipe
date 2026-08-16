# AGENTS.md

Notes for anyone — human or otherwise — changing this repository.

## What this project is

Shaipe is a workspace for SVG assets — logos, marks, visual identities — built
so that a vision-capable agent can work on them. **The project SVG is the
source of truth**: a single file that carries the artwork *and*, in its
`<metadata>`, the prompt, palette, fonts, variants and render specifications
that describe it. Rendering is local, deterministic and completely independent
of any model, so the same project opens in the TUI or regenerates a
repository's assets in CI with no network access at all. Shaipe does not host a
model and never will — it gives an agent that already exists the ability to
*see* an SVG. If you are about to add a provider, an API key or a `generate`
command that calls one, read `docs/adr/004-tools-not-a-model.md` first.

## Non-negotiable invariants

1. **Rendering is deterministic.** Same project bytes, same specification, same
   output bytes, on any machine. No clock, no network, no environment, no
   system fonts unless the document asked for a family the project failed to
   supply — enforced by `rendering_the_same_project_twice_produces_identical_bytes`
   in `src/render/mod.rs`, and end to end by `.github/workflows/assets.yaml`,
   which re-renders `logo.svg` and fails if `docs/images` changed.

2. **Opening a project and saving it changes nothing.** Metadata is spliced
   over its own byte range and defaults are omitted, so Shaipe never dirties a
   working tree just by reading it — enforced by
   `saving_an_untouched_project_would_not_change_a_byte` in `src/project/mod.rs`.

3. **The renderer knows nothing about terminals** — enforced by the
   `renderer-isolation` hook in `prek.toml`.

4. **The preview layer knows nothing about projects or rendering.** It is
   handed pixels and an area — enforced by the `preview-isolation` hook.

5. **Nothing in the library knows a command exists.** `src/cli/` and
   `src/main.rs` are the binary; everything else must be usable without it —
   enforced by the `library-knows-no-commands` hook.

6. **The stdout lock is never held across the workspace.** `Stdout`'s lock is
   re-entrant only for the thread holding it, and `ratatui-image` queries the
   terminal from a spawned thread — a lock held across it silently downgrades
   every preview to half-blocks. `stdout().lock()` appears exactly once, in
   `write_lines`. Enforced by the `stdout-lock-not-held-across-the-tui` hook
   and by `scripts/check-terminal-detection.py`, which runs `shaipe doctor`
   under a pty that impersonates a Kitty terminal.

7. **The project file stays a valid, ordinary SVG.** Its root draws the primary
   variant, so it renders in a browser and on GitHub rather than appearing
   blank. Metadata never affects geometry — by construction, since `usvg` never
   sees it.

## Layout

```
src/
├── lib.rs        the library surface
├── main.rs       the shaipe binary
├── error.rs      the crate's error type
├── logging.rs    where warnings go
├── inspect.rs    the serialisable description of a project
├── cli/          argument types and one module per command
├── project/      the format: document, metadata, palette, variants, specs
├── render/       project + spec -> bytes. Headless, deterministic
├── preview/      pixels -> terminal, via ratatui-image. See ADR 006
├── tui/          the interactive workspace
└── tools/        the operations an agent can perform
```

Dependencies point inward, and the direction is enforced, not described:

```
cli ──> tui ──> preview ──┐
  │       │               ├──> (pixels only)
  └───────┴──> render ────┴──> project ──> error
                tools ────────────┘
```

`project` knows nothing. `render` knows `project`. `preview` knows neither.
Nothing in the library knows a command exists.

## Style

**Every non-obvious line carries a comment saying why.** Not what — the code
says what. Ideally naming the failure it prevents. A comment that restates the
code is worse than none.

**Errors are typed and actionable.** `thiserror` for the library, `miette` at
the binary edge. A diagnostic must carry the two things the user does not
already know: what specifically failed, and what to do about it. When something
names a variant, a spec or a colour that does not exist, say what does.
Diagnostic codes are `shaipe::<module>::<kind>`, and a code is a public
identifier users grep for — renaming one is a breaking change. So is renaming a
tool in `src/tools/`.

**Test names are sentences.** `an_unchanged_input_produces_no_output`, not
`test_run_2`. The name should say what would be broken if it failed.

**Verify the SVG stack; do not assume it.** Two of this project's core design
decisions exist because the obvious API does not do what its name suggests:
`usvg` discards `<metadata>`, and `node_by_id` cannot find a `<symbol>`. Both
were found by probing, not by reading. Do the same before building on an
assumption about `resvg`.

## Plan

`PLAN.md` is the single checklist of what is done and what is not. Its format
is load-bearing and documented in the file: **checkboxes, never numbering**,
items are never reordered or deleted, and finishing something is a
one-character change in place. That is what lets several people edit it at once
without conflicts. Read the rules at the top before changing it.

## Artwork

`logo.svg` is a Shaipe project, and everything in `docs/images/` is rendered
from it by `mise run assets`. Never edit a file in `docs/images/` by hand — CI
will overwrite the change and fail. The artwork is currently a hand-drawn
placeholder; regenerating it *through Shaipe* is the point of the dogfooding
loop and has not happened yet.

## Commits

Conventional Commits, enforced by commitlint on `commit-msg`. The type becomes a
changelog heading, so choose it as if someone will read it in release notes —
because they will.

## Releases

Driven by gh-ship. Never bump a version or push a tag by hand: `cliff.toml`
derives the version from the commit history, `prepare-release` applies it, and
`.github/ship.yml` is the contract between them. See CONTRIBUTING.md.

## Before you push

```sh
mise run ci
```

Formatting, Clippy, spelling, workflow linting, tests and the documentation build. Same as CI.

## This repository is generated from a template

The toolchain, hooks, CI and release workflows come from
[rust.tpl](https://github.com/noirbizarre/rust.tpl) and are updated with
`git tpl update`. Files carrying template-owned content end with a
`# --- project-specific ---` marker: add below it, never above.

Changing template-owned content here fixes it in one repository. Changing it in
the template fixes it in all of them — prefer that.
