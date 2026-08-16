# ADR-002 — The terminal preview is hand-written, behind a trait

## Status

Superseded by [ADR-006](006-ratatui-image.md).

The decision below rested entirely on `ratatui-image` requiring rustc 1.90.
That MSRV is now accepted, and with the premise gone the conclusion does not
stand. Kept as written, because the reasoning is still the right shape — it was
the input that changed, not the argument.

## Context

The workspace has to show what a variant actually looks like. Terminal graphics
are a set of mutually incompatible escape-sequence protocols — Kitty, Sixel,
iTerm2 — with uneven support, and a half-block fallback that works everywhere.

`ratatui-image` solves this. It implements all four protocols, queries the
terminal for its capabilities, and integrates with `ratatui` directly. Using it
was the plan.

Measuring it changed the decision. `ratatui-image` hard-depends on `icy_sixel`,
which depends on `quantette`, which requires **rustc 1.90**. Adopting it would
have raised this project's MSRV by two releases in order to obtain a Sixel
encoder Shaipe does not use, and would additionally have pulled in `image`,
`rand`, `self_cell`, `base64-simd`, `rustix` and `windows` — a large tree for
"draw a PNG in a terminal".

The alternative is not as expensive as it sounds. The Kitty protocol is a
base64 PNG in an APC sequence, chunked at 4096 bytes; the half-block renderer
is one character and two colours per cell.

## Decision

Define a `Preview` trait in `src/preview/` and implement both backends by hand:
`kitty` for terminals that speak the Kitty graphics protocol, and `blocks` for
everything else.

- The trait is the boundary. Sixel and iTerm2 would be new modules and nothing
  else.
- Backend selection reads the environment rather than querying the terminal.
  A query means writing an escape sequence and waiting for a reply, which hangs
  on a terminal that does not answer and is awkward to get right while
  something else is reading the same input. `--preview` overrides the guess.
- **Both backends always draw.** The half-block widget renders into ratatui's
  buffer on every frame, and the Kitty backend then draws over it. A terminal
  that ignores the graphics sequence therefore shows a coarse preview instead
  of an empty box.

## Consequences

- The MSRV stays at 1.88, and the dependency tree stays small: `ratatui` and
  `base64`.
- Capability detection is less precise than a real query. Under `tmux` in
  particular, the passthrough requires `allow-passthrough`, which is off by
  default, so a preview may fail through no fault of this code. `--preview
  blocks` is the escape hatch, and the status line names the backend in use so
  a bad preview is diagnosable without a debugger.
- Kitty's protocol details are now this project's problem: chunking, image ids,
  deleting the previous placement, and doubling escapes inside a tmux
  passthrough. Each is covered by a test that does not need a terminal.
- If a fifth protocol appears, or the Kitty implementation turns out to be
  subtly wrong in a terminal nobody tested, revisiting this in favour of
  `ratatui-image` costs one module — which is the point of the trait.
