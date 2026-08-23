# Plan

Where Shaipe is and where it is going. Checked means done and shipped.

## How to edit this file

Read this before changing anything here, because the format is load-bearing.

- **Checkboxes only. Nothing is ever numbered** — not items, not headings, not
  sub-lists. A number is a position, and a position is what makes two people
  editing the same file collide.
- **Items are never reordered and never deleted.** Finishing something is a
  one-character change, `[ ]` to `[x]`, in place. Reordering is precisely the
  conflict this format exists to avoid.
- **New items are appended to the end of their section**, never inserted.
- **Abandoned work is checked and struck through** with the reason, so the
  decision survives: `- [x] ~~Thing~~ — dropped, see ADR-00N.`
- **One self-contained line per item.** No item may depend on the one above it,
  because neither of them is guaranteed to stay there.
- Sections are stable. Add a new `##` section rather than renaming one.

Anything with a decision behind it belongs in `docs/adr/`, not here. This file
tracks *what*; the ADRs record *why*.

## Format

The project SVG and its metadata.

- [x] `<shaipe:project>` in `<metadata>`, own namespace, versioned schema
- [x] Prompt, palette, fonts, variants, references, render specifications
- [x] Unknown elements and attributes ignored rather than rejected
- [x] Reject a schema version this build does not implement
- [x] Metadata written by splicing its own byte range, artwork untouched
- [x] Defaults omitted on write, so opening and saving changes nothing
- [x] Variants as `<symbol>`, root `<use>` keeps the file viewable
- [ ] Palette *binding* — artwork references palette entries, so editing a
      colour restyles the mark
- [ ] Import and export standard palette formats
- [x] Record `<shaipe:generation>` when an agent actually produces artwork

## Renderer

Local, deterministic, no model.

- [x] `render(project, spec) -> RenderedAsset`
- [x] Variant isolation at the XML level, since `usvg` drops `<symbol>`
- [x] PNG output at any size, aspect-preserving, centred
- [x] SVG output — an isolated variant is already a standalone document
- [x] Transparent and flat-colour backgrounds
- [x] Fonts declared by the project, system fallback only when it must
- [x] `--strict-fonts` refuses the fallback, for CI
- [x] Byte-identical output across runs, machines and optimisation levels
- [x] Metadata stripped from exported SVG — an asset is not a project
- [ ] Padding and inset in a render specification
- [ ] Watermark composition
- [ ] Rendering to JPEG and WebP
- [ ] Prune definitions the isolated variant does not use
- [x] Dependencies optimised in the dev profile — a debug build rasterised
      ~25x slower than release
- [x] `Renderer::pixels` stops before encoding, for callers that show a render
      rather than write one

## CLI

- [x] `shaipe render` — all declared specs, a named subset, or an ad-hoc one
- [x] `shaipe render --dry-run`
- [x] `shaipe inspect`, text and `--format json`
- [x] `shaipe tui`, and bare `shaipe` opening the conventional project
- [x] `shaipe doctor` — terminal, tmux, detected protocol, cell size, failure
- [x] Typed, actionable diagnostics that name what does exist
- [ ] `shaipe palette` — read and edit the palette without the TUI
- [x] `shaipe init` — create a project from nothing
- [ ] Shell completions and a man page

## Workspace

The interactive TUI.

- [x] Four panes: prompt, palette, variants, render specifications
- [x] Live preview through the same renderer `shaipe render` uses
- [x] Kitty, Sixel, iTerm2 and half-block previews via `ratatui-image`
- [x] `--preview` forces a backend when detection is wrong
- [x] A render failure shows the reason instead of taking the terminal down
- [x] Terminal capability query actually reaches the terminal
- [x] The focused pane expands; the others collapse to their titles
- [x] Mouse: click to focus and select, wheel to scroll
- [x] Mouse: drag the column divider to resize
- [x] Mouse: double-click a render specification to export it
- [x] A collapsed pane scrolls, so its selection stays visible
- [x] An SVG render specification is previewed by rasterising it
- [x] The preview is rasterised at the size the pane can show, not the size the
      specification declares
- [x] The preview takes pixels directly, instead of encoding a PNG and decoding
      it again
- [x] Rendering happens off the drawing thread, so the workspace never freezes
      while a preview is produced
- [x] A spinner while a render is in flight, with the previous preview left on
      screen rather than blanked
- [x] The selection is debounced, so arrowing through a list renders where you
      stop rather than everywhere you passed
- [x] Input is drained before drawing, so a burst of keys is one state change
- [x] The workspace renders once at startup, not twice
- [x] `--preview` works without naming a subcommand
- [x] A frame announcing the image is drawn before the write that blocks on it,
      so the workspace looks busy rather than wedged
- [x] The event loop is asynchronous, waiting on the terminal and the
      renderer at once, with a pty guard that the preview still detects Kitty
- [x] Edit the prompt in place — `ratatui-textarea`, the ratatui-org fork of
      `tui-textarea`; the original was ruled out for pinning ratatui 0.29, the
      fork builds on `ratatui-core` and soft-wraps, which the original never did
- [x] `e` on the prompt pane suspends the workspace and opens `$EDITOR` on it
- [x] Save the project from the workspace, with a dirty marker and a guard on
      quitting with unsaved changes
- [ ] A colour picker for the palette pane
- [ ] Add, remove and reorder variants and render specifications
- [ ] Export all assets from the workspace
- [x] A toolbar naming every state the workspace is in, and clickable
- [x] The left column is half the width, prompt over palette at 2:1, and no
      pane grows just because it has the keyboard
- [x] The variants and the render specifications are the preview's tabs, walked
      with `←` and `→`, wrapping at both ends
- [x] `m` swaps which of the two the tabs list, globally
- [x] `s` swaps the preview for the source, globally
- [x] Edit a palette colour's name and value in place, committed as you type,
      with the half-typed text on screen and only a colour reaching the document
- [x] One edit mode for the prompt and the palette: `enter` engages the focused
      pane, `tab` moves between them without leaving it, arrows belong to the
      pane
- [x] A modal render specifications editor — `x` — that adds, removes and
      retypes every field of one
- [x] Mouse: double-click a render specification's tab to export it
- [x] One spinner implementation for rasterising and for the agent, so
      "generating a preview" and "working" animate identically
- [x] The workspace's name is text rather than a button, every button is
      labelled with what pressing it will do, and the arrows move between the
      panes until one of them is being edited
- [ ] Scroll back through the transcript: it is anchored to its newest entry,
      and nothing but `PageUp` reaches the rest of it
- [ ] Render the transcript's Markdown — bold, italics and real bullets —
      rather than showing the characters a model wrote
- [ ] A variant's preview takes the whole pane, rendered to fit the space it
      has, where a render specification keeps being drawn at the size it
      declares
- [ ] Review the toolbar, the footer and the pane titles together: they say
      overlapping things, and which one to read for what is not obvious

## Agent integration

Shaipe provides tools. It does not provide a model, a key or a conversation —
see `docs/adr/004-tools-not-a-model.md`.

- [x] `Tool` trait and `Registry`, transport-agnostic
- [x] `get_project`, `get_variants`, `get_palette`, `render_svg`
- [x] Stable tool names and ordering, descriptions written as prompt text
- [ ] Prove the loop that already works: an agent runs `shaipe render` and
      opens the PNG. Until this is done and written up, the premise is
      unproven
- [x] A transport for the registry — MCP over stdio is the obvious one
- [x] Mutating tools: set a palette colour, write a variant, record generation
- [ ] Tools for attaching and inspecting references
- [ ] Raster to vector: hand a PNG to an agent and get an SVG back
- [x] Tool inputs described by JSON Schema rather than a parameter list — ADR-008
- [x] Tools renamed to `verb_noun`, pinned by a test — ADR-007
- [x] `get_svg` and `write_svg`, the written document validated before it is
      accepted and never saved without being asked
- [x] `render_grid` — one variant at several sizes, to check a mark still reads
- [ ] A `save_project` tool, and a dirty marker an agent can see
- [x] The project is owned by one task and reached by message, so there is no
      lock anywhere in the library — ADR-009
- [x] An MCP server over stdio: `shaipe mcp <project>`, verified against a real
      OpenCode client
- [x] The live workspace serves MCP on a socket, reached by a bridge
      subprocess — ADR-010
- [x] An ACP client: the workspace drives an agent the user already installed,
      with OpenCode as the first target — ADR-011
- [ ] A permission dialogue, so the agent's own tools can be allowed one at a
      time rather than all or nothing
- [ ] Show the agent's plan in the transcript
- [ ] Prove the vision loop against a vision-capable model, and say in the
      README which models can and cannot see a render
- [x] ~~`a` opens a message box for the agent~~ — there is no chat; the prompt
      is the instruction and `a` sends it
- [x] `a` sends the project's prompt to the agent, which makes the artwork
      match it
- [x] A spinner and the running tool in the status line, so a turn is visibly
      happening
- [x] `s` cycles the right-hand column between the preview, the SVG the agent
      would read, and the log of what it did
- [x] The log appears by itself while a turn runs and gives way to the result
- [x] The transcript takes the prompt's box rather than the right-hand column,
      and `t` swaps the two at any moment, during a turn or not
- [x] An agent's edit only re-renders the preview when it changes the variant
      on screen, compared as isolated bytes either side of the call
- [x] Notices expire, so a save stops hiding every key hint for the session
- [x] `alt+a` reaches the agent from inside the editor, which swallows every
      other key
- [x] The project file is watched, so an agent writing it directly still
      reaches the preview, and a save cannot overwrite somebody else's work
- [x] ~~Refuse the agent's own editing tools~~ — not from a client; see ADR-012
- [x] Deny the agent's editing and shell tools through the environment it is
      started in, so `write_svg` is the only way the project changes — ADR-013
- [ ] `Policy::Guarded` is unproven against a real agent that asks: OpenCode
      never does, so only its unit tests cover it
- [ ] Interrupt a turn with a double `esc`, as well as with `ctrl-c`
- [ ] Choose a model with vision and prove the agent sees the rendered SVG
      rather than reading its source — the test that claims this passed once
      against a model that said "I cannot see images" and looked the answer up

## Artwork

- [x] `logo.svg` is a Shaipe project at the repository root
- [x] `docs/images/` rendered from it by `mise run assets`
- [x] CI re-renders and fails if the committed assets drifted
- [x] Light and dark variants share one geometry through `currentColor`
- [ ] Replace the hand-drawn placeholder with artwork generated *through*
      Shaipe — the point of the dogfooding loop, and not yet done

## Performance

Measured, not guessed. `--dry-run` so no writes are timed; best of seven.

- [x] Debug build: all seven assets 1887 ms -> 77 ms
- [x] Preview path at 512²: 63 ms -> 17 ms in a debug build
- [ ] The document is parsed by `roxmltree` N + 2 times for N specs, and the
      source cloned N + 1 times — worth ~2 ms against 15-45 ms of rasterising,
      so measure before touching it
- [x] The workspace rebuilds the font database on every preview miss — the
      render worker now owns one renderer
- [x] Kitty transmits raw RGBA sized to the pane, so one change was ~1.4 MB.
      Halved under tmux, where passthrough makes it slow: 1.46 MB -> 0.38 MB.
      `--preview-scale` overrides it
- [ ] `preview_spec()` clones a `RenderSpec` on every frame to compare it with
      the cache key
- [ ] With no variants, `refresh_preview` allocates an error string every tick

## Project

Toolchain, docs, release.

- [x] Rendered from `rust.tpl`, template updates still tracked
- [x] Architecture guards in `prek.toml`, each checked against a violation
- [x] ADRs for every decision worth re-proposing
- [x] Documentation site
- [ ] First release
- [ ] Publish to crates.io
- [ ] Compatibility matrix for terminals in the README
