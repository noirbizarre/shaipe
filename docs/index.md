# shaipe

An LLM-native SVG asset workspace.

An agent can write SVG markup. What it cannot do is *look* at the result. Shaipe
keeps a logo and everything that describes it — prompt, palette, fonts,
variants, the assets to produce — in one ordinary SVG file, and renders that
file locally, exactly, and without ever calling a model.

Shaipe does not host a model, and will not. Bring your own agent.

## Installation

```bash
cargo install shaipe
```

Or download a binary for your platform from the
[latest release](https://github.com/noirbizarre/shaipe/releases/latest).

## The project file

A Shaipe project is a valid SVG whose `<metadata>` carries a versioned
`<shaipe:project>` element:

```xml
<svg xmlns="http://www.w3.org/2000/svg" xmlns:shaipe="https://shaipe.dev/ns/2026"
     viewBox="0 0 256 256" width="256" height="256">
  <metadata>
    <shaipe:project version="1" primary="icon">
      <shaipe:prompt>A geometric mark, partly drawn and partly inferred.</shaipe:prompt>
      <shaipe:palette>
        <shaipe:color name="accent" value="#f05032" role="accent"/>
      </shaipe:palette>
      <shaipe:variants>
        <shaipe:variant name="icon"/>
      </shaipe:variants>
      <shaipe:renders>
        <shaipe:render name="favicon-32" variant="icon" width="32"/>
      </shaipe:renders>
    </shaipe:project>
  </metadata>

  <symbol id="icon" viewBox="0 0 256 256"><!-- … --></symbol>
  <use href="#icon" width="256" height="256"/>
</svg>
```

Every element is optional except `<shaipe:project version>`. A variant is a
`<symbol>` carrying its own `viewBox`, so it has its own canvas; the root
`<use>` draws the primary variant, so the file is still a logo when a browser
opens it.

Shaipe writes metadata back by replacing exactly the bytes that element
occupied, so opening a project and saving it changes nothing else — not the
artwork, not the comments, not the indentation.

## Init

```bash
shaipe init
```

Creates a minimal project — one variant, `icon`, drawn as the primary — so
there is something to open, render or hand to an agent. Refuses to overwrite
a file that is already there unless `--force` is given; `--prompt "…"` seeds
the prompt. `--source <path>` attaches an existing image as a `source`
reference instead — the thing an agent traces or vectorises — and can be
combined with `--prompt` to say what to keep or change; the file must already
exist. `shaipe tui` and bare `shaipe` do the same automatically for a path
that does not exist yet, in memory, until `ctrl-s` writes it.

## Rendering

```bash
shaipe render logo.svg --output docs/images
shaipe render logo.svg --variant icon --width 64 --output dist
```

With no flags, every declared specification is produced. Rendering reads no
network, no clock and no environment, so the same project bytes produce the
same output bytes anywhere — which is what makes it a CI check:

```yaml
- run: shaipe render logo.svg --output docs/images --strict-fonts
- run: git diff --exit-code -- docs/images
```

`--strict-fonts` refuses the system-font fallback. See
[ADR-005](adr/005-declared-fonts.md) for why that matters.

## Inspecting

```bash
shaipe inspect logo.svg
shaipe inspect logo.svg --format json
```

The JSON form is the stable surface for an agent.

## The workspace

```bash
shaipe            # opens ./logo.svg
```

The project's prompt and palette on the left, a live preview on the right,
drawn with whichever graphics protocol the terminal reports — Kitty, Sixel or
iTerm2 — and Unicode half-blocks everywhere else. A toolbar along the top names
every state the workspace is in and lets you click it.

The variants and the render specifications are the preview's tabs: `←` and `→`
move between them and `m` swaps which of the two they list. `s` swaps the
picture for the SVG that produced it, `t` swaps the prompt for the transcript,
and `x` opens the editor for whichever the tabs currently list — variants or
render specifications, each a table that adds, removes and reorders its rows.
Each of the four is a toolbar button too, labelled with what pressing it will
do rather than with the state it is in.

`tab` and `↑` `↓` move between the prompt and the palette, `enter` hands the
keyboard to whichever has it and `esc` gives it back; both are edited in place and
committed as you type. `e` opens the prompt in `$VISUAL` or `$EDITOR`, and
`ctrl-s` saves; the status line marks unsaved work and quitting with any asks
first. The mouse works: click a button, a tab or a pane, double-click to edit,
wheel to scroll, drag the divider to resize, and double-click a specification's
tab to export it.

```bash
shaipe doctor
```

reports the terminal, the tmux passthrough setting, the detected protocol, the
cell size and the transmit scale — ask it first when a preview looks wrong.
`--preview` forces a specific backend: `auto`, `kitty`, `sixel`, `iterm2` or
`blocks`. Under `tmux`, graphics also need `allow-passthrough`.

Kitty transmits raw pixels, and under tmux each 4 KiB chunk needs its own
passthrough sequence — enough to make a full-resolution preview take seconds.
Previews are therefore transmitted at half resolution when tmux is detected,
and the terminal scales them back up. `--preview-scale N` overrides that.

## Design

The decisions worth arguing about, and the alternatives rejected, are recorded
as [architecture decisions](adr/README.md). Two of them exist because the
obvious `resvg` API does not do what its name suggests.
