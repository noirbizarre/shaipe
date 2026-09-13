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

### Init

```console
$ shaipe init
created logo.svg
```

Creates a minimal project: one variant, `icon`, drawn as the primary so the
file is a viewable SVG from the first byte. Refuses to overwrite an existing
file unless `--force` is given, and `--prompt "…"` seeds the prompt a
workspace or an agent would otherwise be asked to fill in.

`--source <path>` attaches an existing image as a `source` reference — the
thing an agent traces or vectorises, instead of working from a prompt alone.
Combine both to say what to reproduce *and* what to change:

```console
$ shaipe init --source mockup.png --prompt "keep the mark, drop the wordmark"
created logo.svg
```

The file must already exist; `get_reference_image` is how an agent then looks
at it, `get_reference_trace` is how it measures one algorithmically into
vector paths rather than hand-describing them, and `set_reference` attaches
further references, or edits this one, after creation.

`--inspiration <path>` attaches a mood board instead — cues to take, not to
copy — and can be given more than once:

```console
$ shaipe init --inspiration mood-a.png --inspiration mood-b.png --prompt "warmer, rounder"
created logo.svg
```

Both flags resolve from the current directory, the same as any other path on
the command line, but are stored relative to the project itself — so a
project created in a subdirectory, sourced from an image sitting beside the
shell rather than beside it, still finds the image once the shell is gone. A
reference typed inside the workspace or given to `set_reference` has no shell
to resolve from, so there the convention is simply: relative to the project,
or absolute.

`shaipe tui` and bare `shaipe` do this automatically for a path that does not
exist yet — nothing is written until `ctrl-s`.

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
┌ Shaipe  [Transcript] [Source] [Renders] [Edit renders] ───────────────────┐
├───────────────────────────────┬───────────────────────────────────────────┤
│ prompt, or the transcript     │ ‹ icon │ wordmark ›                       │
│                               │                                           │
├───────────────────────────────┤        preview, or the source             │
│ palette                       │                                           │
├───────────────────────────────┤                                           │
│ references                    │                                           │
└───────────────────────────────┴───────────────────────────────────────────┘
```

The preview uses whichever graphics protocol the terminal actually reports —
Kitty, Sixel or iTerm2 — and falls back to Unicode half-blocks everywhere else.
It is produced by the same renderer `shaipe render` uses, so it is the asset
rather than an impression of it.

The variants and the render specifications are the preview's **tabs**: `←` and
`→` move between them, wrapping at both ends, and `m` swaps which of the two the
tabs list. `s` swaps the picture for the SVG that produced it. `t` swaps the
prompt for the transcript. Each of those has a button on the toolbar as well,
because a mode with no visible affordance is one people find by accident.

`tab` and `↑` `↓` move between the prompt, the palette and the references pane,
`enter` hands the keyboard to whichever has it and `esc` gives it back. The
prompt and the palette are edited in place: the prompt is prose, and a palette
colour is its name and its value, committed as you type. The references pane
has no in-place editor — `x` opens a modal table for attaching, retyping and
removing one, the same way it does for variants and render specifications when
the tabs list one of those instead. Every button says what pressing it will do,
so the toolbar reads `Source` while the preview is up and `Preview` while it is
not. `r` re-renders, `q` quits.
The mouse works: click a toolbar button or a tab, click a pane to focus it,
double-click to edit it, wheel to scroll, drag the divider to resize, and
double-click a specification's tab to write it to `dist/`.

`e` opens the prompt in `$VISUAL` or `$EDITOR` instead. `ctrl-s` writes the
project back to its file. The status line marks unsaved work, and quitting with
any asks first.

If the preview looks wrong, ask:

```bash
shaipe doctor
```

It reports the terminal, the tmux passthrough setting, the protocol that was
detected, the cell size and the transmit scale, so a preview problem is
diagnosable instead of mysterious. `--preview blocks` forces the fallback.

Under `tmux`, Kitty images go through a passthrough sequence per 4 KiB chunk,
which is slow enough to be noticeable — and slow *only* there. So previews are
transmitted at half resolution when tmux is detected and the terminal scales
them back up. `--preview-scale N` overrides it: `1` for full sharpness, `4` for
speed.

## Working with an agent

Shaipe does not host a model. It drives an agent you already installed, and
gives that agent the one thing it does not have: the ability to *see* an SVG.

```text
              Shaipe
                 │
                ACP          Shaipe is the client.
                 │           The agent is a program you installed.
             opencode acp
                 │
        your model, your key, your bill
                 │
                MCP          Shaipe is the server.
                 │
         Shaipe's asset tools
```

Shaipe never sees an API key, never names a provider, and cannot tell you which
model answered. Your agent is configured and authenticated in your agent. See
[ADR-011](docs/adr/011-driving-an-agent-is-still-not-a-model.md).

### The tools

| Tool | What it does |
|---|---|
| `get_project` | Everything the project says about itself |
| `get_variants` | The named parts of the document that can be drawn alone |
| `get_palette` | The colours, with their names and roles |
| `get_references` | Files attached for context, and whether they exist |
| `get_reference_image` | Read an attached reference's bytes and look at it |
| `get_reference_trace` | Trace a reference's pixels into vector paths, algorithmically |
| `get_svg` | The document, exactly as it is |
| `render_svg` | Draw one variant and **look at it** |
| `render_grid` | Draw one variant at several sizes, to check it still reads small |
| `write_svg` | Replace the document, validated first |
| `write_variant` | Replace one variant's element, without resending the whole document |
| `set_palette_colour` | Set a colour's value or role by name, or declare a new one |
| `set_reference` | Attach a file for context, or update one already attached |
| `set_generation` | Record which agent and model produced the current state, and when |

The names are a public interface; renaming one is a breaking change
([ADR-007](docs/adr/007-tool-names.md)). `render_svg` and `render_grid` return
real images as MCP image content, which is the entire point — a model that can
only read the SVG cannot tell you the mark is illegible at 16 pixels.

`write_svg`, `write_variant`, `set_palette_colour`, `set_reference` and
`set_generation` all change the project **in memory** only. None of them write
to your working tree; that takes a `Ctrl-S`, or `shaipe mcp --write` if the
agent is the only one using the project.

### Two ways to reach them

**Standalone.** A plain MCP server over stdio, for any MCP client:

```bash
shaipe mcp logo.svg
```

```jsonc
// opencode.json — or the equivalent for Claude Code, an IDE, anything else
{
  "mcp": {
    "shaipe": {
      "type": "local",
      "command": ["shaipe", "mcp", "logo.svg"],
      "enabled": true
    }
  }
}
```

This needs no ACP and no agent. It opens the file itself, and works headlessly.

**In the workspace.** Open a project and talk to it:

```bash
shaipe logo.svg
```

Tab to the prompt pane, press `enter` and describe the artwork you want, then
press `a`. Shaipe starts
`opencode acp`, gives it a session-scoped MCP server pointing at *the project
you are looking at*, and shows you what the agent does with it. When the agent
edits the SVG, the preview follows on its own.

The two modes share every tool and no lifecycle. The standalone server owns a
file; the session server reaches the project someone is watching — which is why
the agent's edits and your preview cannot drift apart
([ADR-010](docs/adr/010-mcp-over-a-socket-with-a-bridge.md)).

```bash
shaipe --agent "some-other-agent acp"     # any ACP agent, not just OpenCode
shaipe --no-agent                         # open the workspace without one
shaipe --yes                              # approve the agent's own tools
shaipe --model "anthropic/claude-opus-4-1" # request one of the agent's own models
```

The keys:

| Key | |
|---|---|
| `enter` | edit the focused pane — the prompt, or a palette colour — committed as you type |
| `esc` | stop editing |
| `tab` / `shift-tab`, `↑` / `↓` | move between the prompt, the palette and the references pane, still editing |
| `a` | **send the prompt to the agent**, so it makes the artwork match |
| `alt+a` | the same, without leaving the editor |
| `e` | open the prompt in `$EDITOR` |
| `←` / `→` | the previous or next tab, wrapping at both ends |
| `m` | swap the variants for the render specifications |
| `s` | swap the preview for the SVG that produced it |
| `t` | swap the prompt for the transcript |
| `x` | open the editor for whichever the tabs list, or for the references pane when that has the keyboard |
| `M` | search and pick from whichever models the agent offers, shown once one is chosen |
| `PageUp` / `PageDown` | scroll the source, or the transcript |
| `ctrl-c` | stop the turn the agent is on; again to quit |
| `ctrl-s` | save the project |
| `R` | re-read the project from disk, discarding what is in memory |

The prompt *is* the instruction. There is no chat: you describe the artwork in
the prompt box and press `a`, and the agent reads the document, rewrites it and
looks at the result.

While it works the box shows the transcript — what it said and which tools it
ran — and when the turn ends it gives the prompt back, by which time the
preview is the new artwork. `t` swaps the two at any moment, during a turn or
not. The footer carries a spinner and the tool it is running.

An edit only re-renders the preview when it changes the variant you are looking
at, so the agent rewriting the wordmark leaves the icon on screen alone.

`a` only works where the prompt actually is — once the editor has the keyboard
it owns every key — so `alt+a` is the one that works from inside it.

Both editors are tables: `↑↓` picks a row, `←→` a field, `ctrl-n` adds a row,
`ctrl-d` removes one, `ctrl+↑`/`ctrl+↓` moves one, and `esc` closes it. Chords
throughout, because `+`, `-` and plain arrows are characters and motions
somebody retyping a field would otherwise expect to reach it. The render
specifications editor has six columns — name, variant, width, height, format,
background; the variants editor has two — name and the id of the element in
the document it draws. Adding a variant aliases the element the selected row
already points at, so the new row renders immediately; retype its element to
point it at something else once one exists.

### What the agent may and may not do

`write_svg` is the only way the project changes. Shaipe starts OpenCode with
its file-editing and shell tools **denied**, so the agent cannot write to your
working tree even if it decides to — and OpenCode removes a denied tool rather
than refusing it, so the model does not propose one and then apologise.

Reading, searching and fetching stay allowed. An agent that can read your
`AGENTS.md` and the SVG it is editing does markedly better work, and none of it
can damage the project.

This is done with the environment the agent is started in, merged into whatever
configuration you already have — your model, provider, MCP servers and plugins
are untouched. The workspace says so on the first turn rather than doing it
quietly. See
[ADR-013](docs/adr/013-restrict-the-agent-through-its-environment.md).

It is not a flag, and `--yes` does not switch it off: `--yes` answers
permission requests, and an explicit denial is not a request.

Shaipe only knows how to do this for OpenCode. Any other agent is started
unrestricted, and the workspace says that too. Restrict it in its own
configuration, and the file watching below still applies either way — the
project is watched, so an agent that writes it directly still updates the
preview, and unsaved work is never silently overwritten.

If OpenCode is not installed, the workspace still opens and says so in the
prompt pane. Reading your own project has never depended on an agent, and it
does not start now.

### Prerequisites

[OpenCode](https://opencode.ai), installed and authenticated:

```bash
opencode auth login
```

Any ACP agent works; OpenCode is the one this was built and tested against. A
model that can accept images is needed for `render_svg` to be worth anything —
without one the image is still delivered, and the model still cannot see it.

If your agent's default model is not one, `--model`/`SHAIPE_MODEL` (or `M` in
the workspace, once a session is open) asks it to switch to one that is —
matched against whatever the agent's own selector already offers, never
chosen by Shaipe. See [ADR-011](docs/adr/011-driving-an-agent-is-still-not-a-model.md).

## Architecture

```text
cli ──> tui ──> preview ──┐
  │       │               ├──> (pixels only)
  │       ├──> render ────┴──> project ──> error
  │       │                       ▲
  │       └──> acp ──┐            │
  └──────────> mcp ──┴──> tools ──┘
```

- **`project`** — the format. Reads and writes the SVG and its metadata, and
  knows nothing else.
- **`render`** — project + specification → bytes. Headless and deterministic;
  contains no reference to a terminal or a model.
- **`preview`** — pixels → terminal. Knows nothing about SVG.
- **`tui`** — the workspace.
- **`tools`** — the operations an agent can perform. The application layer:
  no transport, and the only place any of them is implemented.
- **`mcp`** — those tools, spoken as the Model Context Protocol.
- **`acp`** — an Agent Client Protocol client, for driving an agent.

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
- `shaipe render`, `shaipe inspect`, `shaipe init`, and a CI workflow that
  regenerates this repository's own artwork from `logo.svg` and fails if it
  drifted.
- The terminal workspace, with Kitty, Sixel, iTerm2 and half-block previews.
- Seven tools, over MCP: `shaipe mcp` serves any MCP client, and the workspace
  serves a session-scoped server to an agent it drives over ACP. Verified
  against OpenCode 1.18.18 — it calls `get_variants` and answers with the
  project's real names, and `render_svg` puts a genuine PNG on the wire.
- Talking to an agent from the workspace: type a prompt, watch the tool calls,
  and see the preview follow the agent's edit.

**Not yet**

- Any generation *by Shaipe*. There is no `shaipe generate`, no provider and no
  API key, and adding one is
  [explicitly not the plan](docs/adr/004-tools-not-a-model.md). Shaipe drives
  an agent you own; it does not become one.
- A proven vision loop. The image is delivered as MCP image content and a real
  agent receives it, but whether the *model* can see it depends on the model
  you configured. The end-to-end test skips loudly rather than pretending
  otherwise when it cannot.
- A permission dialogue. The agent's own tools — reading files, running
  commands — are refused unless `--yes` is passed, because there is nothing to
  ask with yet. Shaipe's own tools never ask.
- Editing from the workspace beyond the prompt: the palette pane has no colour
  picker yet.
- Palette *binding*. The palette is recorded and reported, but the artwork does
  not yet reference it, so editing a colour does not restyle the mark.

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
