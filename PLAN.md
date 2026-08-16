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
- [ ] Record `<shaipe:generation>` when an agent actually produces artwork

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

## CLI

- [x] `shaipe render` — all declared specs, a named subset, or an ad-hoc one
- [x] `shaipe render --dry-run`
- [x] `shaipe inspect`, text and `--format json`
- [x] `shaipe tui`, and bare `shaipe` opening the conventional project
- [x] `shaipe doctor` — terminal, tmux, detected protocol, cell size, failure
- [x] Typed, actionable diagnostics that name what does exist
- [ ] `shaipe palette` — read and edit the palette without the TUI
- [ ] `shaipe init` — create a project from nothing
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
- [ ] Edit the prompt in place — needs a text widget; `tui-textarea` is ruled
      out (pins ratatui 0.29), `edtui` fits but is Vim-modal, so confirm first
- [ ] `E` suspends the workspace and opens `$EDITOR` on the prompt
- [ ] Save the project from the workspace, with a dirty marker and a guard on
      quitting with unsaved changes
- [ ] A colour picker for the palette pane
- [ ] Add, remove and reorder variants and render specifications
- [ ] Export all assets from the workspace

## Agent integration

Shaipe provides tools. It does not provide a model, a key or a conversation —
see `docs/adr/004-tools-not-a-model.md`.

- [x] `Tool` trait and `Registry`, transport-agnostic
- [x] `inspect_project`, `list_variants`, `inspect_palette`, `render`
- [x] Stable tool names and ordering, descriptions written as prompt text
- [ ] Prove the loop that already works: an agent runs `shaipe render` and
      opens the PNG. Until this is done and written up, the premise is
      unproven
- [ ] A transport for the registry — MCP over stdio is the obvious one
- [ ] Mutating tools: set a palette colour, write a variant, record generation
- [ ] Tools for attaching and inspecting references
- [ ] Raster to vector: hand a PNG to an agent and get an SVG back

## Artwork

- [x] `logo.svg` is a Shaipe project at the repository root
- [x] `docs/images/` rendered from it by `mise run assets`
- [x] CI re-renders and fails if the committed assets drifted
- [x] Light and dark variants share one geometry through `currentColor`
- [ ] Replace the hand-drawn placeholder with artwork generated *through*
      Shaipe — the point of the dogfooding loop, and not yet done

## Project

Toolchain, docs, release.

- [x] Rendered from `rust.tpl`, template updates still tracked
- [x] Architecture guards in `prek.toml`, each checked against a violation
- [x] ADRs for every decision worth re-proposing
- [x] Documentation site
- [ ] First release
- [ ] Publish to crates.io
- [ ] Compatibility matrix for terminals in the README
