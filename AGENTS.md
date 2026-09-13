# AGENTS.md

Notes for anyone — human or otherwise — changing this repository.

## What this project is

Shaipe is a workspace for SVG assets — logos, marks, visual identities — built
so that a vision-capable agent can work on them. **The project SVG is the
source of truth**: a single file that carries the artwork *and*, in its
`<metadata>`, the prompt, palette, fonts, variants and render specifications
that describe it. Rendering is local, deterministic and completely independent
of any model, so the same project opens in the TUI or regenerates a
repository's assets in CI with no network access needed beyond a warm font
cache (see below). Shaipe does not host a model and never will — it gives an
agent that already exists the ability to *see* an SVG. If you are about to
add a provider, an API key or a `generate` command that calls one, read
`docs/adr/004-tools-not-a-model.md` first.

## Non-negotiable invariants

1. **Rendering is deterministic.** Same project bytes, same specification, same
   output bytes, on any machine. No clock, no environment, no system fonts
   unless the document asked for a family the project failed to supply, and
   no network *reads* — the one narrow exception is a font declared by a
   checksum-pinned URL (`docs/adr/015-checksum-pinned-remote-fonts.md`),
   fetched once into a local cache and never touched again on a match; a
   render itself still never makes the request, `crate::fonts` does, before
   `src/render/` ever sees the bytes — enforced by
   `rendering_the_same_project_twice_produces_identical_bytes`
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

7. **Nothing but JSON-RPC reaches standard output while `shaipe mcp` serves.**
   A single `log::warn!` about a missing font in the middle of the stream is a
   frame the client cannot parse, and it looks like Shaipe speaking a broken
   protocol rather than like a warning. `logging::suppress()` is held for the
   whole session — the stdio twin of invariant 6 — and
   `nothing_but_json_rpc_is_written_to_standard_output` in
   `tests/mcp_stdio.rs` runs the binary under `-vv` and parses every line.

8. **The workspace never waits for an agent's handshake.** An agent starts the
   MCP servers it is given in `session/new` and calls `tools/list` on them
   *before answering it*. Shaipe's server is the live workspace, which can only
   answer from its event loop, so waiting deadlocks the two: the agent times
   out, and the session comes up with none of Shaipe's tools and no error
   anywhere. `Agent::start` returns immediately and reports through the update
   stream — enforced by `starting_an_agent_does_not_wait_for_it_to_answer` in
   `src/acp/agent.rs`, and explained in ADR 011.

9. **There is no lock anywhere in the library.** The project has one owner and
   is reached by message; see ADR 009. This is what invariant 6's hook is
   really protecting now, and the ADR records why spelling a lock
   `RwLock::write()` to evade that hook was rejected.

10. **The project file stays a valid, ordinary SVG.** Its root draws the primary
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
├── tools/        the operations an agent can perform. The application layer
├── mcp/          those tools, spoken as MCP. A thin adapter
└── acp/          an ACP client, for driving an agent. A thin adapter
```

Dependencies point inward, and the direction is enforced, not described:

```
cli ──> tui ──> preview ──┐
  │       │               ├──> (pixels only)
  │       ├──> render ────┴──> project ──> error
  │       │                       ▲
  │       └──> acp ──┐            │
  └──────────> mcp ──┴──> tools ──┘
```

`project` knows nothing. `render` knows `project`. `preview` knows neither.
Nothing in the library knows a command exists. `mcp` and `acp` are adapters
around `tools`, and neither knows the workspace exists — enforced by the
`adapter-isolation` hook, because `shaipe mcp` has to work headlessly.

Two moving protocols are confined to the modules that adapt them, by the
`acp-types-are-confined` and `mcp-types-are-confined` hooks. That is what makes
`AgentUpdate` and `ToolOutput` load-bearing rather than decorative: ACP's
`SessionUpdate` is `#[non_exhaustive]`, and a pane that matched on it would
stop compiling every time the protocol grew.

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

**Probe the protocols too.** The same rule cost more here than anywhere else.
`agent-client-protocol`'s major version is not the protocol's — 2.x speaks wire
version 1. `tempfile::tempdir` does not create a private directory; its mode is
whatever the umask leaves. And an agent calls `tools/list` on a server it was
given *before* answering the request that gave it. Each of those was a
plausible assumption, each was wrong, and each was found by running the thing.
`opencode acp` will answer a hand-written JSON-RPC frame on stdin; use it.

## Plan

`PLAN.md` is the single checklist of what is done and what is not. Its format
is load-bearing and documented in the file: **checkboxes, never numbering**,
items are never reordered or deleted, and finishing something is a
one-character change in place. That is what lets several people edit it at once
without conflicts. Read the rules at the top before changing it.

## Agents, and the two MCP modes

`shaipe mcp <project>` opens a file and serves it on stdio. The workspace
serves a *session-scoped* server for the project on screen: it listens on a
unix socket and hands the agent `shaipe mcp --bridge <address>`, a subprocess
that copies bytes and parses nothing. They share every tool and no lifecycle;
do not merge them. See ADR 010.

The tools live in `src/tools/` and only there. `src/mcp/` is a transport, and a
tool implemented in it is one the workspace and the CLI cannot reach.

Testing anything in this area against a real agent costs money and needs
someone's credentials, so it is `#[ignore]`d (`tests/acp_opencode.rs`) or a
script (`scripts/smoke-acp.sh`). Everything up to the model is covered by tests
that always run: `src/mcp/server.rs` drives a real MCP client over a duplex,
`src/mcp/bridge.rs` carries a call over a real socket, and `tests/mcp_stdio.rs`
runs the binary. Never add a test that needs a network to `mise run ci`.

The prompt pane's editor serves two purposes and they must not be confused.
`enter` edits the project's `<shaipe:prompt>`, which is metadata and is
committed to the document on every keystroke; `a` composes a message to the
agent, which is committed nowhere. A single buffer with a mode rather than two
widgets, because one pane with two text fields in it is worse than one pane
with two states. `EditorMode` is what keeps them apart, and
`a_message_to_the_agent_never_reaches_the_projects_prompt` is what stops them
merging back.

An ACP client cannot restrict an agent — permission is resolved inside the
agent, and OpenCode's defaults never ask, so `src/acp/`'s `Policy` is correct
code a default install never reaches. But Shaipe *spawns* the agent, and a
parent chooses its child's environment: `src/acp/opencode.rs` starts OpenCode
with `edit` and `bash` denied, merged into whatever config the user already
has. That is the only product-specific module in the crate, and it earns it.

ADR 012 concluded the opposite and is superseded by ADR 013. The mistake is
worth remembering: it reasoned about the protocol and concluded about the
system. `src/tui/watch.rs` is still the backstop, for agents Shaipe cannot
restrict and for a text editor in another window.

`--yes` grants the *agent's* own tools — its editor, its shell — and an agent
asked to change a colour may use them instead of `write_svg`, writing to the
working tree past every guarantee above. Shaipe's own tools never ask and never
save. Do not conflate the two when documenting either.

When a test asserts something about a model's behaviour, make sure it can fail.
`an_agent_can_see_the_artwork_rather_than_only_read_it` originally passed
against a model that said "I cannot see images" and then looked the answer up
with another tool. A green test that proves nothing is worse than no test.

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
