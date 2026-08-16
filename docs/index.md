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

A four-pane description of the project on the left, a live preview on the
right, drawn with whichever graphics protocol the terminal reports — Kitty,
Sixel or iTerm2 — and Unicode half-blocks everywhere else. `--preview` forces a
specific one: `auto`, `kitty`, `sixel`, `iterm2` or `blocks`. The fallback is
occasionally necessary under `tmux`, where graphics need `allow-passthrough`.

## Design

The decisions worth arguing about, and the alternatives rejected, are recorded
as [architecture decisions](adr/README.md). Two of them exist because the
obvious `resvg` API does not do what its name suggests.
