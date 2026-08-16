<p align="center">
  <img src="docs/images/icon.svg" alt="shaipe" width="160">
</p>

<h1 align="center">shaipe</h1>

<p align="center"><strong>An LLM-native SVG asset workspace</strong></p>

<p align="center">
  <a href="https://github.com/noirbizarre/shaipe/actions/workflows/ci.yaml">
    <img src="https://github.com/noirbizarre/shaipe/actions/workflows/ci.yaml/badge.svg" alt="CI">
  </a>
  <a href="https://codecov.io/gh/noirbizarre/shaipe">
    <img src="https://codecov.io/gh/noirbizarre/shaipe/graph/badge.svg" alt="Codecov">
  </a>
  <a href="https://crates.io/crates/shaipe">
    <img src="https://img.shields.io/crates/v/shaipe" alt="crates.io">
  </a>
  <img src="https://img.shields.io/github/v/release/noirbizarre/shaipe" alt="Release">
  <a href="https://noirbizarre.github.io/shaipe/">
    <img src="https://img.shields.io/badge/docs-noirbizarre.github.io-blue" alt="Documentation">
  </a>
  <img src="https://img.shields.io/github/license/noirbizarre/shaipe" alt="License">
</p>

---

An agent can write SVG markup. What it cannot do is *look* at the result — tell
whether the mark still reads at 16 pixels, whether the wordmark is legible, or
what the project's variants and colours are even called. Shaipe closes that gap.
It keeps a logo and everything that describes it — prompt, palette, fonts,
variants, the assets to produce — in one ordinary SVG file, and renders that
file locally, exactly and without ever calling a model. The same project opens
in a terminal workspace with a live preview, or regenerates a repository's
entire asset set in CI on a machine with no network.

Shaipe does not host a model, and will not. Bring your own agent.

## The idea

**The project SVG is the source of truth.** Not a sidecar `shaipe.toml` next to
it — the same file. A sidecar can be half-copied, half-renamed and separately
edited, and an agent editing the artwork cannot see it.

```xml
<svg xmlns="http://www.w3.org/2000/svg" xmlns:shaipe="https://shaipe.dev/ns/2026"
     viewBox="0 0 256 256" width="256" height="256">
  <metadata>
    <shaipe:project version="1" primary="icon">
      <shaipe:prompt>A geometric mark, partly drawn and partly inferred.</shaipe:prompt>
      <shaipe:palette>
        <shaipe:color name="accent" value="#f05032" role="accent"/>
        <shaipe:color name="ink"    value="#18181b" role="foreground"/>
      </shaipe:palette>
      <shaipe:variants>
        <shaipe:variant name="icon"/>
        <shaipe:variant name="wordmark"/>
      </shaipe:variants>
      <shaipe:renders>
        <shaipe:render name="favicon-32" variant="icon" width="32"/>
        <shaipe:render name="social" variant="wordmark" width="1280" height="640" background="#18181b"/>
      </shaipe:renders>
    </shaipe:project>
  </metadata>

  <symbol id="icon" viewBox="0 0 256 256"><!-- … --></symbol>
  <symbol id="wordmark" viewBox="0 0 800 256"><!-- … --></symbol>

  <!-- The root draws the primary variant, so this file is still a logo. -->
  <use href="#icon" width="256" height="256"/>
</svg>
```

It is a real SVG. A browser renders it, GitHub renders it, Inkscape opens it,
and every tool that has never heard of Shaipe ignores a namespace it does not
know. Nothing in `<metadata>` can affect the geometry, because the renderer's
parser never sees it.

## Installation

```bash
cargo install shaipe
```

Or download a binary for your platform from the
[latest release](https://github.com/noirbizarre/shaipe/releases/latest).

## Usage

### Render

With no other flags, `render` produces every asset the project declares:

```console
$ shaipe render logo.svg --output docs/images
docs/images/icon.svg (256x256)
docs/images/favicon-32.png (32x32)
docs/images/icon-128.png (128x128)
docs/images/icon-512.png (512x512)
docs/images/wordmark-512.png (512x160)
docs/images/wordmark-dark-512.png (512x160)
docs/images/social.png (1280x640)
```

Or one asset described entirely on the command line:

```bash
shaipe render logo.svg --variant icon --width 64 --output dist
```

No network, no model, no clock, no environment. The same project bytes produce
the same output bytes on any machine, which is what makes this a CI step:

```yaml
- run: shaipe render logo.svg --output docs/images --strict-fonts
- run: git diff --exit-code -- docs/images
```

`--strict-fonts` refuses to fall back to the system's fonts. A system font
would render something plausible on the runner and something different on a
laptop — silently.

### Inspect

```console
$ shaipe inspect logo.svg
logo.svg

  A mark for Shaipe, an LLM-native SVG asset workspace…

variants
  icon             #icon  (primary)
  wordmark         #wordmark

palette
  accent           #f05032    accent
  ink              #18181b    foreground
```

`--format json` is the same report, and is the surface an agent should read.

### Workspace

```bash
shaipe            # opens ./logo.svg
shaipe tui logo.svg
```

```text
┌───────────────────────┬───────────────────────────┐
│ prompt                │                           │
│ palette               │         preview           │
│ variants              │                           │
│ render specs          │                           │
└───────────────────────┴───────────────────────────┘
```

The preview uses whichever graphics protocol the terminal actually reports —
Kitty, Sixel or iTerm2 — and falls back to Unicode half-blocks everywhere else.
It is produced by the same renderer `shaipe render` uses, so it is the asset
rather than an impression of it.

`tab` moves between panes and the focused pane takes the column, `↑↓` selects,
`r` re-renders, `q` quits. The mouse works: click to focus and select, wheel to
scroll, drag the divider to resize, double-click a render specification to
write it to `dist/`.

If the preview looks wrong, ask:

```bash
shaipe doctor
```

It reports the terminal, the tmux passthrough setting, the protocol that was
detected and the cell size, so a preview problem is diagnosable instead of
mysterious. `--preview blocks` forces the fallback.

## Architecture

```text
cli ──> tui ──> preview ──┐
  │       │               ├──> (pixels only)
  └───────┴──> render ────┴──> project ──> error
                tools ────────────┘
```

- **`project`** — the format. Reads and writes the SVG and its metadata, and
  knows nothing else.
- **`render`** — project + specification → bytes. Headless and deterministic;
  contains no reference to a terminal or a model.
- **`preview`** — pixels → terminal. Knows nothing about SVG.
- **`tui`** — the workspace.
- **`tools`** — the operations an agent can perform, with no transport.

Those directions are enforced by hooks in `prek.toml`, not merely documented.
The reasoning behind each significant choice — including two where the obvious
`resvg` API turned out not to do what its name suggests — is in
[docs/adr](docs/adr).

## Status

Early, but real. Nothing described above is a mock.

**Works today**

- The project format: metadata, palette, fonts, variants, references, render
  specifications, with a versioned schema and byte-preserving writes.
- Deterministic rendering to PNG and SVG, at any size, with backgrounds.
- `shaipe render`, `shaipe inspect`, and a CI workflow that regenerates this
  repository's own artwork from `logo.svg` and fails if it drifted.
- The terminal workspace, with Kitty, Sixel, iTerm2 and half-block previews.
- A read-only tool registry: `inspect_project`, `list_variants`,
  `inspect_palette`, `render` — **as a library API only. It has no transport,
  so nothing outside this crate can call it yet.** What works today is an
  agent running `shaipe render` and opening the PNG.

**Not yet**

- Any generation. There is no `shaipe generate`, no provider and no API key,
  and adding one is [explicitly not the plan](docs/adr/004-tools-not-a-model.md).
- A transport for the tool registry. MCP is the obvious next step. Until then
  the registry is a library API with no external consumer.
- Editing from the workspace: the prompt pane displays, the palette pane has no
  colour picker yet.
- Palette *binding*. The palette is recorded and reported, but the artwork does
  not yet reference it, so editing a colour does not restyle the mark.
- Raster → vector. PNG and JPEG references can be attached to a project; asking
  a model to reconstruct an SVG from one is future work.

**Dogfooding.** `logo.svg` at the root of this repository is a Shaipe project,
and every image in `docs/images/` is rendered from it. The artwork itself is
still a hand-drawn placeholder — regenerating it *through Shaipe* is the point,
and has not happened yet.

## Plan

[PLAN.md](PLAN.md) tracks what is done and what is not, in one checklist.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT — see [LICENSE](LICENSE).
