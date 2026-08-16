# ADR-006 — Use `ratatui-image` for terminal previews

## Status

Accepted. Supersedes [ADR-002](002-preview-backends.md).

## Context

ADR-002 rejected `ratatui-image` and hand-wrote the Kitty and half-block
backends. The reasoning was sound at the time and rested on a single fact:
`ratatui-image` hard-depends on `icy_sixel` → `quantette`, which requires
**rustc 1.90**. Adopting it meant raising the MSRV by two releases to obtain a
Sixel encoder Shaipe did not use.

That premise no longer holds. 1.90 is stable and long since released, and the
project owner accepts it as the floor. Once the MSRV objection is removed,
nothing else in ADR-002 survives contact with what the hand-written code
actually cost:

- **It implemented two protocols out of four.** Sixel and iTerm2 were listed as
  "a new module and nothing else", which was true and also never going to
  happen. Users of `foot`, `mlterm`, `WezTerm` and iTerm2 got half-blocks.
- **Capability detection was a guess.** ADR-002 chose environment sniffing over
  querying the terminal, on the grounds that a query can hang. `ratatui-image`
  queries with a timeout, which is the answer that had been assumed away — and
  it additionally knows that Konsole's Sixel support is broken and that WezTerm
  should prefer iTerm2, neither of which was going to be discovered here.
- **Out-of-band drawing infected the architecture.** Writing escape sequences
  straight to stdout meant `ui::draw` had to return the preview's `Rect` so the
  caller could paint over its own frame afterwards, and the event loop had to
  sequence a clear, a draw, and an overlay in the right order. All of that
  existed to work around not being a widget.

## Decision

Depend on `ratatui-image`, and delete the hand-written protocol code.

- `src/preview/` keeps its module boundary and its own `Image` and `Backend`
  types. `ratatui-image` is named nowhere else in the crate, and the
  `preview-isolation` hook still enforces that nothing in the layer reaches a
  project or the renderer.
- `Backend` mirrors `ProtocolType` rather than re-exporting it, so `--preview`
  keeps a stable spelling independent of that crate's naming.
- Detection is `Picker::from_query_stdio`, called after the alternate screen is
  up and before any event is read. Failure degrades to half-blocks.
- Default features stay **off**, with only `crossterm` enabled: the default set
  includes `chafa-dyn`, which links a system library through `pkg-config` and
  is not present on a stock CI runner.
- The preview is a `StatefulWidget` like everything else, so `ui::draw` returns
  nothing and the event loop draws one frame.

## Consequences

- **The MSRV rises to 1.90**, and `Cargo.toml` says why.
- Sixel and iTerm2 work now, for free, and so does anything `ratatui-image`
  adds later.
- Roughly 300 lines of protocol code and its tests are gone — chunking, image
  ids, placement, tmux escape doubling. Those tests were passing; they were
  testing an implementation that did not need to exist.
- The half-block fallback flattens transparency, because a terminal cell has no
  alpha. It flattens against mid grey rather than the default black, since
  black makes dark artwork — including Shaipe's own ink `#18181b` — invisible.
  Only the fallback does this; the real protocols carry alpha through.
- One capability is lost: ADR-002 drew half-blocks *and* the graphics protocol
  on every frame, so a terminal that ignored the escape sequence still showed a
  coarse image. `ratatui-image` picks one. This is the right trade — the case
  it protected against is a terminal that both claims a protocol and ignores
  it, and `--preview blocks` remains the escape hatch.
- A protocol that renders a uniform region emits a coloured *space*, not `▀`.
  Any test asserting an image was drawn must look at cell background colours;
  asserting on glyphs silently passes on nothing. This cost one debugging
  session and is recorded so it costs no more.
