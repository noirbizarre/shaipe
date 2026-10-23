# ADR-032 — Select, copy and paste inside a workspace that owns the mouse

## Status

Accepted

## Context

The workspace captures the mouse so panes can be clicked and dividers dragged.
A terminal that has given its mouse away has no text selection of its own, so
nothing on screen could be selected or copied; the one workaround, Shift-drag,
was mentioned in a code comment and nowhere a user would read it. Nothing wrote
to the system clipboard either — the editors' `ctrl-c` filled the text area's
private yank buffer — and bracketed paste was never enabled, so a paste arrived
as keystrokes: slow, committed per character, with an `esc` or tab in it
leaving the pane, and with no editor open every letter a shortcut.

## Decision

- **Paste.** Bracketed paste is always on and `Event::Paste` is handled by
  `App::paste`: inserted whole into the description or message editor (one
  commit), typed into the palette field or a modal's text field with newlines
  flattened, and ignored when nothing is being edited.
- **Copy.** Text reaches the clipboard through OSC 52 (`tui::clipboard`), not
  a clipboard crate: a crate talks to the display server of the machine the
  process runs on, which is the wrong one over SSH, and it adds a native
  dependency. The cost is that a terminal which refuses OSC 52 refuses silently.
- **Select.** A drag inside a pane selects what is drawn there. It reads the
  finished frame, not each pane's model (`tui::selection`), so every pane works
  with one implementation and what is copied is what was highlighted; the price
  is that a soft-wrapped line copies as the rows it was drawn in. The
  selection is bounded by the pane's interior, so it never crosses a border.
- **Escape hatch.** `F2` (and `--no-mouse`) releases mouse capture so the
  terminal's own selection works, which is the answer for everything the
  workspace's selection cannot do — across panes, inside modals — and for a
  terminal that ignores OSC 52. It is handled before any editor or modal.

## Consequences

Mouse capture stays, so the clickable interface and the dividers are
unchanged. The status line names `F2`, the one thing a user stuck without a
selection cannot guess. Reading the clipboard is deliberately not offered: a
terminal's paste is the user's act, and a program that reads the clipboard on
its own is a different trust decision. The OSC 52 payload is capped
(`clipboard::MAX_BYTES`) because terminals drop an over-long sequence whole.
